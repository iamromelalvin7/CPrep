//! The "Today" dashboard: activity, streak, progress, review, practice, interviews, spend.

use anyhow::Result;
use chrono::{Duration, Local, NaiveDate};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::BTreeMap;

use crate::{db, review, schedule};

#[derive(Serialize)]
pub struct DayCount {
    pub date: String,
    pub count: i64,
}

#[derive(Serialize)]
pub struct TrackProgress {
    pub id: &'static str,
    pub name: &'static str,
    pub accent: &'static str,
    pub done: i64,
    pub total: i64,
}

#[derive(Serialize)]
pub struct InterviewPoint {
    pub id: i64,
    pub kind: String,
    pub date: String,
    pub overall: i64,
}

#[derive(Serialize)]
pub struct Spend {
    pub total_usd: f64,
    pub claude_usd: f64,
    pub month_usd: f64,
    pub budget_usd: f64,
    pub by_model: Vec<(String, f64, i64)>,
}

#[derive(Serialize)]
pub struct NextLesson {
    pub track_id: String,
    pub lesson_id: i64,
    pub title: String,
    pub status: String,
}

#[derive(Serialize)]
pub struct Dashboard {
    pub activity: Vec<DayCount>,
    pub streak: i64,
    pub active_days_30: i64,
    pub tracks: Vec<TrackProgress>,
    pub review: review::ReviewSummary,
    pub retention_30d: Option<f64>,
    pub exercises_tried: i64,
    pub exercises_solved: i64,
    pub interviews: Vec<InterviewPoint>,
    pub upcoming: Vec<schedule::Occurrence>,
    pub next_lesson: Option<NextLesson>,
    pub spend: Spend,
}

pub fn budget_usd() -> f64 {
    std::env::var("CLAUDE_BUDGET_USD").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(25.0)
}

/// Count of learning actions per local day: lesson replies, card reviews, code submissions, interview answers.
pub fn activity(conn: &Connection, days: i64) -> Result<Vec<DayCount>> {
    let today = Local::now().date_naive();
    let from = today - Duration::days(days - 1);
    let mut counts: BTreeMap<NaiveDate, i64> = BTreeMap::new();
    let mut stmt = conn.prepare(
        "SELECT substr(t, 1, 10) AS d, COUNT(*) FROM (
            SELECT created_at AS t FROM messages WHERE role = 'user' AND content <> ''
            UNION ALL SELECT reviewed_at FROM reviews
            UNION ALL SELECT created_at FROM attempts
            UNION ALL SELECT created_at FROM interview_messages WHERE role = 'user'
         ) WHERE substr(t, 1, 10) >= ?1 GROUP BY d",
    )?;
    let rows = stmt.query_map([from.to_string()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    for row in rows {
        let (d, n) = row?;
        if let Ok(date) = NaiveDate::parse_from_str(&d, "%Y-%m-%d") {
            counts.insert(date, n);
        }
    }
    Ok((0..days)
        .map(|i| {
            let d = from + Duration::days(i);
            DayCount { date: d.to_string(), count: *counts.get(&d).unwrap_or(&0) }
        })
        .collect())
}

/// Consecutive active days ending today (or yesterday, so the streak survives until you study today).
pub fn streak(activity: &[DayCount]) -> i64 {
    let mut days = activity.iter().rev().peekable();
    if days.peek().map(|d| d.count == 0).unwrap_or(false) {
        days.next();
    }
    days.take_while(|d| d.count > 0).count() as i64
}

pub fn build(conn: &Connection) -> Result<Dashboard> {
    let act = activity(conn, 84)?;
    let streak = streak(&act);
    let active_days_30 = act.iter().rev().take(30).filter(|d| d.count > 0).count() as i64;

    let mut tracks = Vec::new();
    for t in db::TRACKS.iter().filter(|t| t.status == "active") {
        let (total, done): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(status IN ('done','skipped')), 0) FROM lessons
             WHERE curriculum_id = (SELECT MAX(id) FROM curricula WHERE track_id = ?1)",
            [t.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if total > 0 {
            tracks.push(TrackProgress { id: t.id, name: t.name, accent: t.accent, done, total });
        }
    }

    let since = (Local::now() - Duration::days(30)).to_rfc3339();
    let (reviewed, recalled): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(rating > 1), 0) FROM reviews WHERE reviewed_at >= ?1",
        [&since],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    let (exercises_tried, exercises_solved): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(best = total), 0) FROM (
            SELECT MAX(passed) AS best, MAX(total) AS total FROM attempts GROUP BY exercise_id)",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    let interviews = conn
        .prepare("SELECT id, kind, started_at, debrief FROM interviews WHERE debrief IS NOT NULL ORDER BY id DESC LIMIT 10")?
        .query_map([], |r| {
            let deb: String = r.get(3)?;
            let started: String = r.get(2)?;
            Ok(InterviewPoint {
                id: r.get(0)?,
                kind: r.get(1)?,
                date: started.chars().take(10).collect(),
                overall: serde_json::from_str::<serde_json::Value>(&deb).ok().and_then(|v| v["overall"].as_i64()).unwrap_or(0),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .rev()
        .collect();

    let now = Local::now();
    let upcoming = schedule::agenda(conn, now - Duration::minutes(30), now + Duration::days(7))?
        .into_iter()
        .filter(|o| chrono::DateTime::parse_from_rfc3339(&o.end).map(|e| e > now).unwrap_or(true))
        .take(4)
        .collect();

    // Continue where you left off: the lesson you last spoke in, or the first unfinished one.
    let next_lesson = conn
        .query_row(
            "SELECT c.track_id, l.id, l.title, l.status FROM messages m
             JOIN lessons l ON l.id = m.lesson_id JOIN curricula c ON c.id = l.curriculum_id
             WHERE l.status <> 'done' ORDER BY m.id DESC LIMIT 1",
            [],
            |r| Ok(NextLesson { track_id: r.get(0)?, lesson_id: r.get(1)?, title: r.get(2)?, status: r.get(3)? }),
        )
        .ok()
        .or_else(|| {
            conn.query_row(
                "SELECT c.track_id, l.id, l.title, l.status FROM lessons l JOIN curricula c ON c.id = l.curriculum_id
                 WHERE l.status IN ('todo','in_progress') ORDER BY c.id DESC, l.ord LIMIT 1",
                [],
                |r| Ok(NextLesson { track_id: r.get(0)?, lesson_id: r.get(1)?, title: r.get(2)?, status: r.get(3)? }),
            )
            .ok()
        });

    let month_start = format!("{}", now.format("%Y-%m-01"));
    let by_model: Vec<(String, f64, i64)> = conn
        .prepare("SELECT model, SUM(cost_usd), COUNT(*) FROM usage GROUP BY model ORDER BY SUM(cost_usd) DESC, COUNT(*) DESC")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let one = |sql: &str, p: &[&dyn rusqlite::ToSql]| -> rusqlite::Result<f64> { conn.query_row(sql, p, |r| r.get(0)) };
    let spend = Spend {
        total_usd: one("SELECT COALESCE(SUM(cost_usd), 0) FROM usage", &[])?,
        claude_usd: one("SELECT COALESCE(SUM(cost_usd), 0) FROM usage WHERE provider = 'anthropic'", &[])?,
        month_usd: one("SELECT COALESCE(SUM(cost_usd), 0) FROM usage WHERE created_at >= ?1", &[&month_start])?,
        budget_usd: budget_usd(),
        by_model,
    };

    Ok(Dashboard {
        activity: act,
        streak,
        active_days_30,
        tracks,
        review: review::summary(conn)?,
        retention_30d: (reviewed > 0).then(|| recalled as f64 / reviewed as f64),
        exercises_tried,
        exercises_solved,
        interviews,
        upcoming,
        next_lesson,
        spend,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn days(counts: &[i64]) -> Vec<DayCount> {
        counts.iter().enumerate().map(|(i, c)| DayCount { date: format!("d{i}"), count: *c }).collect()
    }

    #[test]
    fn streak_counts_back_from_today_or_yesterday() {
        assert_eq!(streak(&days(&[1, 0, 2, 3, 1])), 3, "today counts");
        assert_eq!(streak(&days(&[1, 2, 3, 0])), 3, "a quiet today doesn't break yesterday's streak yet");
        assert_eq!(streak(&days(&[1, 0, 0])), 0);
        assert_eq!(streak(&days(&[])), 0);
    }

    #[test]
    fn dashboard_builds_on_an_empty_database() {
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        let d = build(&conn).unwrap();
        assert_eq!(d.activity.len(), 84);
        assert_eq!(d.streak, 0);
        assert!(d.tracks.is_empty());
        assert!(d.retention_30d.is_none());
        assert_eq!(d.spend.budget_usd, budget_usd());
    }
}
