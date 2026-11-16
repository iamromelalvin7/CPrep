//! Spaced-repetition review cards scheduled with FSRS (v4.5 formulas, default weights).
//! Cards come from lessons: the model writes question/answer pairs from what was covered.

use anyhow::{bail, Result};
use chrono::{DateTime, Duration, Local};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Mutex;

use crate::db::{self, Track};
use crate::llm::Provider;
use crate::tutor;

// ---------- FSRS ----------

const W: [f64; 17] = [
    0.4872, 1.4003, 3.7145, 13.8206, 5.1618, 1.2298, 0.8975, 0.031, 1.6474, 0.1367, 1.0461, 2.1072, 0.0793, 0.3246,
    1.587, 0.2272, 2.8755,
];
const DECAY: f64 = -0.5;
const FACTOR: f64 = 19.0 / 81.0;
/// Target probability of remembering a card when it comes due.
pub const DESIRED_RETENTION: f64 = 0.9;
const MAX_INTERVAL_DAYS: f64 = 365.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rating {
    Again = 1,
    Hard = 2,
    Good = 3,
    Easy = 4,
}

impl Rating {
    pub fn from_u8(n: u8) -> Result<Self> {
        Ok(match n {
            1 => Rating::Again,
            2 => Rating::Hard,
            3 => Rating::Good,
            4 => Rating::Easy,
            _ => bail!("rating must be 1-4"),
        })
    }
    fn g(self) -> f64 {
        self as u8 as f64
    }
}

/// Memory state of one card.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Memory {
    pub stability: f64,
    pub difficulty: f64,
    pub reps: i64,
    pub lapses: i64,
}

pub fn retrievability(elapsed_days: f64, stability: f64) -> f64 {
    (1.0 + FACTOR * elapsed_days / stability).powf(DECAY)
}

fn interval_days(stability: f64) -> f64 {
    (stability / FACTOR * (DESIRED_RETENTION.powf(1.0 / DECAY) - 1.0)).clamp(1.0, MAX_INTERVAL_DAYS)
}

fn init_difficulty(g: f64) -> f64 {
    (W[4] - (g - 3.0) * W[5]).clamp(1.0, 10.0)
}

fn next_difficulty(d: f64, g: f64) -> f64 {
    let d2 = d - W[6] * (g - 3.0);
    (W[7] * init_difficulty(3.0) + (1.0 - W[7]) * d2).clamp(1.0, 10.0)
}

fn recall_stability(d: f64, s: f64, r: f64, rating: Rating) -> f64 {
    let hard = if rating == Rating::Hard { W[15] } else { 1.0 };
    let easy = if rating == Rating::Easy { W[16] } else { 1.0 };
    s * (1.0 + W[8].exp() * (11.0 - d) * s.powf(-W[9]) * ((W[10] * (1.0 - r)).exp() - 1.0) * hard * easy)
}

fn forget_stability(d: f64, s: f64, r: f64) -> f64 {
    (W[11] * d.powf(-W[12]) * ((s + 1.0).powf(W[13]) - 1.0) * (W[14] * (1.0 - r)).exp()).min(s)
}

/// Apply a rating. Returns the new memory state and when the card is next due.
/// Failed cards come back in 10 minutes; everything else is scheduled in whole days.
pub fn schedule(mem: Memory, last_review: Option<DateTime<Local>>, now: DateTime<Local>, rating: Rating) -> (Memory, DateTime<Local>) {
    let g = rating.g();
    let mut next = mem;
    next.reps += 1;
    if mem.reps == 0 || mem.stability <= 0.0 {
        next.stability = W[(g as usize) - 1];
        next.difficulty = init_difficulty(g);
    } else {
        let elapsed = last_review.map(|t| (now - t).num_seconds().max(0) as f64 / 86_400.0).unwrap_or(0.0);
        let r = retrievability(elapsed, mem.stability);
        next.difficulty = next_difficulty(mem.difficulty, g);
        next.stability = if rating == Rating::Again {
            next.lapses += 1;
            forget_stability(mem.difficulty, mem.stability, r)
        } else {
            recall_stability(mem.difficulty, mem.stability, r, rating)
        };
    }
    let due = if rating == Rating::Again {
        now + Duration::minutes(10)
    } else {
        now + Duration::days(interval_days(next.stability).round() as i64)
    };
    (next, due)
}

/// Human preview for each button, e.g. "10m", "3d", "2mo".
pub fn preview(mem: Memory, last_review: Option<DateTime<Local>>, now: DateTime<Local>) -> Vec<String> {
    [Rating::Again, Rating::Hard, Rating::Good, Rating::Easy]
        .into_iter()
        .map(|r| {
            let (_, due) = schedule(mem, last_review, now, r);
            let mins = (due - now).num_minutes();
            if mins < 60 {
                format!("{mins}m")
            } else if mins < 60 * 24 {
                format!("{}h", mins / 60)
            } else if mins < 60 * 24 * 45 {
                format!("{}d", (mins as f64 / 1440.0).round())
            } else {
                format!("{}mo", (mins as f64 / 43_200.0).round())
            }
        })
        .collect()
}

// ---------- Storage ----------

#[derive(Clone, Debug, Serialize)]
pub struct Card {
    pub id: i64,
    pub track_id: String,
    pub lesson_id: Option<i64>,
    pub lesson_title: Option<String>,
    pub front: String,
    pub back: String,
    pub due: String,
    pub memory: Memory,
    pub last_review: Option<String>,
    /// Interval previews for Again, Hard, Good, Easy.
    pub preview: Vec<String>,
}

fn parse_time(s: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Local))
}

fn card_from_row(r: &rusqlite::Row) -> rusqlite::Result<Card> {
    let memory = Memory { stability: r.get(7)?, difficulty: r.get(8)?, reps: r.get(9)?, lapses: r.get(10)? };
    let last_review: Option<String> = r.get(11)?;
    let lr = last_review.as_deref().and_then(parse_time);
    Ok(Card {
        id: r.get(0)?,
        track_id: r.get(1)?,
        lesson_id: r.get(2)?,
        lesson_title: r.get(3)?,
        front: r.get(4)?,
        back: r.get(5)?,
        due: r.get(6)?,
        memory,
        last_review,
        preview: preview(memory, lr, Local::now()),
    })
}

const CARD_COLS: &str = "c.id, c.track_id, c.lesson_id, l.title, c.front, c.back, c.due, c.stability, c.difficulty, c.reps, c.lapses, c.last_review";

/// Cards due now, oldest first. `track` filters to one track.
pub fn due_cards(conn: &Connection, track: Option<&str>, limit: usize) -> Result<Vec<Card>> {
    let now = Local::now().to_rfc3339();
    let mut stmt = conn.prepare(&format!(
        "SELECT {CARD_COLS} FROM cards c LEFT JOIN lessons l ON l.id = c.lesson_id
         WHERE c.due <= ?1 AND (?2 IS NULL OR c.track_id = ?2) AND c.suspended = 0
         ORDER BY c.due LIMIT ?3"
    ))?;
    let rows = stmt.query_map(params![now, track, limit as i64], card_from_row)?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

#[derive(Clone, Debug, Serialize)]
pub struct ReviewSummary {
    pub due_now: i64,
    pub total: i64,
    pub new_cards: i64,
    pub reviewed_today: i64,
    pub next_due: Option<String>,
    pub by_track: Vec<(String, i64)>,
}

pub fn summary(conn: &Connection) -> Result<ReviewSummary> {
    let now = Local::now();
    let start_of_day = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_local_timezone(Local).unwrap().to_rfc3339();
    let now_s = now.to_rfc3339();
    let one = |sql: &str, p: &[&dyn rusqlite::ToSql]| -> rusqlite::Result<i64> { conn.query_row(sql, p, |r| r.get(0)) };
    let by_track = conn
        .prepare("SELECT track_id, COUNT(*) FROM cards WHERE due <= ?1 AND suspended = 0 GROUP BY track_id")?
        .query_map([&now_s], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(ReviewSummary {
        due_now: one("SELECT COUNT(*) FROM cards WHERE due <= ?1 AND suspended = 0", &[&now_s])?,
        total: one("SELECT COUNT(*) FROM cards", &[])?,
        new_cards: one("SELECT COUNT(*) FROM cards WHERE reps = 0", &[])?,
        reviewed_today: one("SELECT COUNT(*) FROM reviews WHERE reviewed_at >= ?1", &[&start_of_day])?,
        next_due: conn
            .query_row("SELECT MIN(due) FROM cards WHERE due > ?1 AND suspended = 0", [&now_s], |r| r.get(0))?,
        by_track,
    })
}

pub fn rate(conn: &Connection, card_id: i64, rating: Rating) -> Result<String> {
    let (mem, last): (Memory, Option<String>) = conn.query_row(
        "SELECT stability, difficulty, reps, lapses, last_review FROM cards WHERE id = ?1",
        [card_id],
        |r| Ok((Memory { stability: r.get(0)?, difficulty: r.get(1)?, reps: r.get(2)?, lapses: r.get(3)? }, r.get(4)?)),
    )?;
    let now = Local::now();
    let (next, due) = schedule(mem, last.as_deref().and_then(parse_time), now, rating);
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE cards SET stability = ?1, difficulty = ?2, reps = ?3, lapses = ?4, due = ?5, last_review = ?6 WHERE id = ?7",
        params![next.stability, next.difficulty, next.reps, next.lapses, due.to_rfc3339(), now.to_rfc3339(), card_id],
    )?;
    tx.execute(
        "INSERT INTO reviews (card_id, rating, reviewed_at) VALUES (?1, ?2, ?3)",
        params![card_id, rating as u8, now.to_rfc3339()],
    )?;
    tx.commit()?;
    Ok(due.to_rfc3339())
}

pub fn insert_cards(conn: &Connection, track_id: &str, lesson_id: Option<i64>, cards: &[(String, String)]) -> Result<usize> {
    let now = Local::now().to_rfc3339();
    let tx = conn.unchecked_transaction()?;
    let mut n = 0;
    for (front, back) in cards {
        // Skip exact duplicates of an existing card in this lesson.
        let dup: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM cards WHERE track_id = ?1 AND front = ?2)",
            params![track_id, front],
            |r| r.get(0),
        )?;
        if dup {
            continue;
        }
        tx.execute(
            "INSERT INTO cards (track_id, lesson_id, front, back, due, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![track_id, lesson_id, front, back, now],
        )?;
        n += 1;
    }
    tx.commit()?;
    Ok(n)
}

#[derive(Clone, Debug, Serialize)]
pub struct LessonCards {
    pub lesson_id: i64,
    pub title: String,
    pub track_id: String,
    pub cards: i64,
    pub status: String,
}

/// Lessons that have been started, with how many cards each has.
pub fn lessons_for_cards(conn: &Connection) -> Result<Vec<LessonCards>> {
    let mut stmt = conn.prepare(
        "SELECT l.id, l.title, c.track_id, (SELECT COUNT(*) FROM cards k WHERE k.lesson_id = l.id), l.status
         FROM lessons l JOIN curricula c ON c.id = l.curriculum_id
         WHERE l.status IN ('in_progress', 'done') AND EXISTS(SELECT 1 FROM messages m WHERE m.lesson_id = l.id)
         ORDER BY l.ord",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(LessonCards { lesson_id: r.get(0)?, title: r.get(1)?, track_id: r.get(2)?, cards: r.get(3)?, status: r.get(4)? })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

// ---------- Generation ----------

fn cards_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "cards": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {"front": {"type": "string"}, "back": {"type": "string"}},
                    "required": ["front", "back"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["cards"],
        "additionalProperties": false
    })
}

pub async fn generate(db: &Mutex<Connection>, provider: &Provider, track: &Track, lesson_id: i64) -> Result<(usize, crate::llm::Usage)> {
    let (lesson, transcript) = {
        let conn = db.lock().unwrap();
        let lesson = tutor::get_lesson(&conn, lesson_id)?;
        let mut t = String::new();
        for m in tutor::messages(&conn, lesson_id)? {
            if m.content.trim().is_empty() {
                continue;
            }
            t.push_str(if m.role == "user" { "\nLEARNER: " } else { "\nTUTOR: " });
            t.push_str(&m.content);
        }
        if t.len() > 20_000 {
            let mut cut = t.len() - 20_000;
            while !t.is_char_boundary(cut) {
                cut += 1;
            }
            t = t[cut..].to_string();
        }
        (lesson, t)
    };
    if transcript.trim().is_empty() {
        bail!("Start the lesson first. Cards are made from what you covered.");
    }
    let system = "You write spaced-repetition flashcards for a student learning software engineering. \
Make 5-8 cards from what this session actually taught. Each card tests ONE idea. The front is a specific question \
(not \"What is X?\" trivia when a 'why', 'when' or 'what happens if' question works better; short code-reading questions \
are welcome). The back is the answer in 1-3 sentences or a few lines of code. Use Markdown for code. No emoji.";
    let user = format!(
        "Track: {}\nLesson: {}\nTopics: {}\n\nSession transcript:\n{}",
        track.name,
        lesson.title,
        lesson.objectives.join("; "),
        transcript
    );
    let (v, usage) = provider.complete_json(system, &user, cards_schema()).await?;
    let cards: Vec<(String, String)> = v["cards"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| (tutor::loose_text(&c["front"]), tutor::loose_text(&c["back"])))
        .filter(|(f, b)| f.len() > 3 && !b.is_empty())
        .take(10)
        .collect();
    if cards.is_empty() {
        bail!("The model didn't return any usable cards. Try again.");
    }
    let conn = db.lock().unwrap();
    db::record_usage(&conn, &usage)?;
    let n = insert_cards(&conn, track.id, Some(lesson_id), &cards)?;
    Ok((n, usage))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> DateTime<Local> {
        Local::now()
    }

    #[test]
    fn new_card_intervals_are_ordered() {
        let now = t0();
        let mem = Memory::default();
        let days: Vec<i64> = [Rating::Hard, Rating::Good, Rating::Easy]
            .iter()
            .map(|r| (schedule(mem, None, now, *r).1 - now).num_days())
            .collect();
        assert!(days[0] <= days[1] && days[1] < days[2], "{days:?}");
        let (_, again) = schedule(mem, None, now, Rating::Again);
        assert_eq!((again - now).num_minutes(), 10);
        // FSRS initial stability for Good is ~3.7 days, so the first interval is about 4 days.
        assert_eq!(days[1], 4);
    }

    #[test]
    fn successful_reviews_grow_intervals_and_lapses_shrink_them() {
        let mut now = t0();
        let (mut mem, mut due) = schedule(Memory::default(), None, now, Rating::Good);
        let mut last = now;
        let mut gaps = vec![];
        for _ in 0..4 {
            now = due;
            let (m, d) = schedule(mem, Some(last), now, Rating::Good);
            gaps.push((d - now).num_days());
            mem = m;
            due = d;
            last = now;
        }
        assert!(gaps.windows(2).all(|w| w[1] > w[0]), "intervals should grow: {gaps:?}");
        let before = mem.stability;
        let (after, d) = schedule(mem, Some(last), due, Rating::Again);
        assert!(after.stability < before);
        assert_eq!(after.lapses, 1);
        assert_eq!((d - due).num_minutes(), 10);
    }

    #[test]
    fn retrievability_at_interval_matches_target() {
        let s = 12.0;
        let r = retrievability(interval_days(s), s);
        assert!((r - DESIRED_RETENTION).abs() < 0.01, "{r}");
    }

    #[test]
    fn cards_store_rate_and_come_back() {
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        let n = insert_cards(&conn, "backend", None, &[("Q1".into(), "A1".into()), ("Q1".into(), "dup".into()), ("Q2".into(), "A2".into())]).unwrap();
        assert_eq!(n, 2, "duplicate fronts are skipped");
        assert_eq!(due_cards(&conn, None, 10).unwrap().len(), 2);
        let card = due_cards(&conn, Some("backend"), 10).unwrap().remove(0);
        assert_eq!(card.preview.len(), 4);
        rate(&conn, card.id, Rating::Good).unwrap();
        assert_eq!(due_cards(&conn, None, 10).unwrap().len(), 1, "a Good card leaves today's queue");
        let s = summary(&conn).unwrap();
        assert_eq!((s.due_now, s.total, s.reviewed_today), (1, 2, 1));
        assert!(s.next_due.is_some());
    }
}
