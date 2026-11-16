//! Planned study sessions and the reminder loop (Windows notifications before each session,
//! plus a daily nudge when review cards are due).

use anyhow::{bail, Result};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    None,
    Daily,
    Weekdays,
    Weekly,
}

impl Repeat {
    fn parse(s: &str) -> Repeat {
        match s {
            "daily" => Repeat::Daily,
            "weekdays" => Repeat::Weekdays,
            "weekly" => Repeat::Weekly,
            _ => Repeat::None,
        }
    }
    fn as_str(&self) -> &'static str {
        match self {
            Repeat::None => "none",
            Repeat::Daily => "daily",
            Repeat::Weekdays => "weekdays",
            Repeat::Weekly => "weekly",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: i64,
    pub track_id: String,
    pub lesson_id: Option<i64>,
    pub lesson_title: Option<String>,
    pub title: String,
    /// Local wall-clock start of the first occurrence, "YYYY-MM-DDTHH:MM".
    pub starts_at: String,
    pub duration_min: i64,
    pub repeat: Repeat,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NewSession {
    pub track_id: String,
    pub lesson_id: Option<i64>,
    pub title: String,
    pub starts_at: String,
    pub duration_min: i64,
    pub repeat: Repeat,
}

#[derive(Clone, Debug, Serialize)]
pub struct Occurrence {
    pub session: Session,
    /// Local start of this occurrence, RFC 3339.
    pub start: String,
    pub end: String,
}

const FMT: &str = "%Y-%m-%dT%H:%M";

pub fn parse_local(s: &str) -> Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s, FMT).or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")).map_err(|_| anyhow::anyhow!("bad date/time: {s}"))
}

fn to_local(n: NaiveDateTime) -> DateTime<Local> {
    Local.from_local_datetime(&n).earliest().unwrap_or_else(|| Local.from_utc_datetime(&n))
}

/// All starts of a session within [from, to).
pub fn occurrences_between(s: &Session, from: DateTime<Local>, to: DateTime<Local>) -> Vec<DateTime<Local>> {
    let Ok(first) = parse_local(&s.starts_at) else { return vec![] };
    let first = to_local(first);
    if s.repeat == Repeat::None {
        return if first >= from && first < to { vec![first] } else { vec![] };
    }
    let mut out = Vec::new();
    let time = first.time();
    let mut day: NaiveDate = from.date_naive().max(first.date_naive());
    // Look one day back so an occurrence that started just before `from` isn't missed by date rounding.
    day = day.pred_opt().unwrap_or(day).max(first.date_naive());
    while to_local(day.and_time(time)) < to {
        let keep = match s.repeat {
            Repeat::Daily => true,
            Repeat::Weekdays => day.weekday().num_days_from_monday() < 5,
            Repeat::Weekly => day.weekday() == first.weekday(),
            Repeat::None => false,
        };
        let at = to_local(day.and_time(time));
        if keep && at >= from && at >= first {
            out.push(at);
        }
        day = day.succ_opt().unwrap();
    }
    out
}

fn session_from_row(r: &rusqlite::Row) -> rusqlite::Result<Session> {
    let repeat: String = r.get(7)?;
    Ok(Session {
        id: r.get(0)?,
        track_id: r.get(1)?,
        lesson_id: r.get(2)?,
        lesson_title: r.get(3)?,
        title: r.get(4)?,
        starts_at: r.get(5)?,
        duration_min: r.get(6)?,
        repeat: Repeat::parse(&repeat),
    })
}

pub fn list(conn: &Connection) -> Result<Vec<Session>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.track_id, s.lesson_id, l.title, s.title, s.starts_at, s.duration_min, s.repeat
         FROM sessions s LEFT JOIN lessons l ON l.id = s.lesson_id ORDER BY s.starts_at",
    )?;
    let rows = stmt.query_map([], session_from_row)?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub fn add(conn: &Connection, s: &NewSession) -> Result<i64> {
    parse_local(&s.starts_at)?;
    if s.title.trim().is_empty() {
        bail!("Give the session a name.");
    }
    if !(5..=480).contains(&s.duration_min) {
        bail!("Sessions can be 5 to 480 minutes long.");
    }
    conn.execute(
        "INSERT INTO sessions (track_id, lesson_id, title, starts_at, duration_min, repeat, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![s.track_id, s.lesson_id, s.title.trim(), s.starts_at, s.duration_min, s.repeat.as_str(), crate::db::now()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
    Ok(())
}

/// Occurrences of every session within [from, to), in time order.
pub fn agenda(conn: &Connection, from: DateTime<Local>, to: DateTime<Local>) -> Result<Vec<Occurrence>> {
    let mut out: Vec<Occurrence> = list(conn)?
        .into_iter()
        .flat_map(|s| {
            occurrences_between(&s, from, to)
                .into_iter()
                .map(|at| Occurrence {
                    start: at.to_rfc3339(),
                    end: (at + Duration::minutes(s.duration_min)).to_rfc3339(),
                    session: s.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    out.sort_by(|a, b| a.start.cmp(&b.start));
    Ok(out)
}

// ---------- Settings ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReminderSettings {
    /// Minutes before a session to notify, e.g. [30, 15, 0].
    pub offsets: Vec<i64>,
    /// Daily "cards are due" nudge at this local time ("HH:MM"), or None.
    pub review_time: Option<String>,
    /// Keep running in the tray when the window is closed.
    pub keep_in_tray: bool,
    pub autostart: bool,
}

impl Default for ReminderSettings {
    fn default() -> Self {
        ReminderSettings { offsets: vec![30, 15, 0], review_time: Some("19:00".into()), keep_in_tray: true, autostart: false }
    }
}

pub fn load_settings(conn: &Connection) -> ReminderSettings {
    conn.query_row("SELECT value FROM settings WHERE key = 'reminders'", [], |r| r.get::<_, String>(0))
        .optional()
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(conn: &Connection, s: &ReminderSettings) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES ('reminders', ?1) ON CONFLICT(key) DO UPDATE SET value = ?1",
        [serde_json::to_string(s)?],
    )?;
    Ok(())
}

// ---------- Reminder decisions ----------

#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    /// Unique key so each reminder fires once: "session:<id>:<start>:<offset>" or "review:<date>".
    pub key: String,
    pub title: String,
    pub body: String,
}

fn starts_in_text(offset: i64) -> String {
    match offset {
        0 => "starts now".into(),
        m if m < 60 => format!("starts in {m} min"),
        m => format!("starts in {} h {} min", m / 60, m % 60),
    }
}

/// What should fire at `now`. A reminder is due during the 2 minutes after its moment, so a short sleep or a
/// slow tick doesn't skip it, but an app opened hours later doesn't spam old reminders.
pub fn due_alerts(
    sessions: &[Session],
    track_name: impl Fn(&str) -> String,
    settings: &ReminderSettings,
    due_cards: i64,
    now: DateTime<Local>,
) -> Vec<Alert> {
    let window = Duration::minutes(2);
    let max_offset = settings.offsets.iter().copied().max().unwrap_or(0);
    let mut out = vec![];
    for s in sessions {
        for start in occurrences_between(s, now - window, now + Duration::minutes(max_offset) + window) {
            for &off in &settings.offsets {
                let moment = start - Duration::minutes(off);
                if now >= moment && now < moment + window {
                    let what = s.lesson_title.clone().unwrap_or_else(|| s.title.clone());
                    out.push(Alert {
                        key: format!("session:{}:{}:{}", s.id, start.format(FMT), off),
                        title: format!("{} {}", track_name(&s.track_id), starts_in_text(off)),
                        body: format!("{what} · {} min. Open CPrep when you're ready.", s.duration_min),
                    });
                }
            }
        }
    }
    if let Some(t) = settings.review_time.as_deref().and_then(|t| NaiveTime::parse_from_str(t, "%H:%M").ok()) {
        let moment = to_local(now.date_naive().and_time(t));
        if due_cards > 0 && now >= moment && now < moment + Duration::hours(3) {
            out.push(Alert {
                key: format!("review:{}", now.date_naive()),
                title: format!("{due_cards} review card{} due", if due_cards == 1 { "" } else { "s" }),
                body: "A few minutes of review keeps what you learned. Open CPrep → Review.".into(),
            });
        }
    }
    out
}

/// Record an alert as sent. Returns false if it was already sent.
pub fn mark_sent(conn: &Connection, key: &str) -> Result<bool> {
    let n = conn.execute("INSERT OR IGNORE INTO alerts_sent (key, sent_at) VALUES (?1, ?2)", params![key, crate::db::now()])?;
    Ok(n == 1)
}

/// The next session occurrence after `now`, for the tray menu.
pub fn next_occurrence(conn: &Connection, now: DateTime<Local>) -> Result<Option<Occurrence>> {
    Ok(agenda(conn, now, now + Duration::days(8))?.into_iter().next())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Local> {
        to_local(parse_local(s).unwrap())
    }

    fn session(start: &str, repeat: Repeat) -> Session {
        Session {
            id: 1,
            track_id: "networking".into(),
            lesson_id: None,
            lesson_title: Some("Subnetting".into()),
            title: "Networking".into(),
            starts_at: start.into(),
            duration_min: 45,
            repeat,
        }
    }

    #[test]
    fn repeats_expand_correctly() {
        // 2026-11-16 is a Monday.
        let from = at("2026-11-16T00:00");
        let to = at("2026-11-23T00:00");
        assert_eq!(occurrences_between(&session("2026-11-16T19:00", Repeat::Daily), from, to).len(), 7);
        assert_eq!(occurrences_between(&session("2026-11-16T19:00", Repeat::Weekdays), from, to).len(), 5);
        assert_eq!(occurrences_between(&session("2026-11-18T19:00", Repeat::Weekly), from, to).len(), 1);
        assert_eq!(occurrences_between(&session("2026-11-20T08:30", Repeat::None), from, to).len(), 1);
        assert!(occurrences_between(&session("2026-12-01T08:30", Repeat::Daily), from, to).is_empty(), "not before the first day");
    }

    #[test]
    fn reminders_fire_at_30_15_and_0_once_each() {
        let s = vec![session("2026-11-16T19:00", Repeat::Daily)];
        let cfg = ReminderSettings::default();
        let name = |_: &str| "Networking".to_string();
        let fire = |t: &str| due_alerts(&s, name, &cfg, 0, at(t));
        assert_eq!(fire("2026-11-16T18:30")[0].title, "Networking starts in 30 min");
        assert_eq!(fire("2026-11-16T18:45")[0].title, "Networking starts in 15 min");
        assert_eq!(fire("2026-11-16T19:00")[0].title, "Networking starts now");
        assert!(fire("2026-11-16T18:50").is_empty());
        assert!(fire("2026-11-16T19:05").is_empty(), "nothing after the window");
        assert_eq!(fire("2026-11-17T18:46")[0].key, "session:1:2026-11-17T19:00:15", "repeats get their own keys");
        assert!(fire("2026-11-16T18:45")[0].body.contains("Subnetting"));
    }

    #[test]
    fn review_nudge_only_when_cards_are_due() {
        let cfg = ReminderSettings::default();
        let none: Vec<Session> = vec![];
        let name = |_: &str| String::new();
        assert!(due_alerts(&none, name, &cfg, 0, at("2026-11-16T19:10")).is_empty());
        let a = due_alerts(&none, name, &cfg, 12, at("2026-11-16T19:10"));
        assert_eq!(a[0].title, "12 review cards due");
        assert!(due_alerts(&none, name, &cfg, 12, at("2026-11-16T17:00")).is_empty());
    }

    #[test]
    fn alerts_are_recorded_once() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init(&conn).unwrap();
        assert!(mark_sent(&conn, "k").unwrap());
        assert!(!mark_sent(&conn, "k").unwrap());
    }
}
