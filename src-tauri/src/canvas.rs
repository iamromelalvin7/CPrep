//! Whiteboard per lesson: saved scene, AI drawing tasks, and diagram critique.
//!
//! The UI describes the drawing as text (shapes, labels, arrows between them), so text-only models
//! can critique it. Vision-capable providers also receive a PNG of the canvas.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::db::{self, Track};
use crate::tutor::Lesson;

#[derive(Clone, Debug, Serialize, Default)]
pub struct CanvasState {
    pub scene: Option<String>,
    pub task: Option<String>,
    pub critique: Option<String>,
}

pub fn load(conn: &Connection, lesson_id: i64) -> Result<CanvasState> {
    Ok(conn
        .query_row("SELECT scene, task, critique FROM canvases WHERE lesson_id = ?1", [lesson_id], |r| {
            Ok(CanvasState { scene: r.get(0)?, task: r.get(1)?, critique: r.get(2)? })
        })
        .optional()?
        .unwrap_or_default())
}

pub fn save_scene(conn: &Connection, lesson_id: i64, scene: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO canvases (lesson_id, scene, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(lesson_id) DO UPDATE SET scene = ?2, updated_at = ?3",
        params![lesson_id, scene, db::now()],
    )?;
    Ok(())
}

pub fn save_task(conn: &Connection, lesson_id: i64, task: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO canvases (lesson_id, task, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(lesson_id) DO UPDATE SET task = ?2, critique = NULL, updated_at = ?3",
        params![lesson_id, task, db::now()],
    )?;
    Ok(())
}

pub fn save_critique(conn: &Connection, lesson_id: i64, critique: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO canvases (lesson_id, critique, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(lesson_id) DO UPDATE SET critique = ?2, updated_at = ?3",
        params![lesson_id, critique, db::now()],
    )?;
    Ok(())
}

pub fn task_prompt(track: &Track, lesson: &Lesson) -> (String, String) {
    let topic = lesson
        .objectives
        .get(lesson.topic_idx.max(0) as usize)
        .cloned()
        .unwrap_or_else(|| lesson.title.clone());
    let system = "You set short drawing exercises on a whiteboard for a student learning backend, systems and AI \
engineering. The student draws boxes, labels and arrows. Pick something a diagram genuinely helps with for this topic: \
a request flow, an architecture, a data structure's shape, a state machine, a database schema, a pipeline. \
Reply in Markdown, under 90 words: one line naming what to draw, then 3-5 bullet requirements the drawing must show. \
No emoji. Don't draw it for them.";
    let user = format!(
        "Track: {}\nLesson: {} (module {})\nCurrent topic: {topic}\nAll topics: {}",
        track.name,
        lesson.title,
        lesson.module,
        lesson.objectives.join("; ")
    );
    (system.into(), user)
}

pub fn critique_prompt(track: &Track, lesson: &Lesson, task: Option<&str>, description: &str, has_image: bool) -> (String, String) {
    let system = "You review a student's whiteboard diagram like a senior engineer in a design interview: direct, \
specific, kind. Reply in Markdown under 220 words:\n\
1. **What works** (1-2 bullets).\n2. **Missing or wrong** (the most important issues first; name the box or arrow).\n\
3. **Next step**: one concrete change to make on the canvas.\n\
Judge against the task if there is one. Arrows mean data or control flow from the first box to the second. \
If the drawing is nearly empty, say what to start with. Don't redraw the whole solution. No emoji.";
    let user = format!(
        "Track: {}\nLesson: {} (module {})\nTopics: {}\n\nTask: {}\n\nThe drawing, as structure{}:\n{}",
        track.name,
        lesson.title,
        lesson.module,
        lesson.objectives.join("; "),
        task.unwrap_or("(no task set; the student drew freely about this lesson)"),
        if has_image { " (an image of the canvas is attached too)" } else { "" },
        if description.trim().is_empty() { "(empty canvas)" } else { description },
    );
    (system.into(), user)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_state_round_trips() {
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO curricula (id, track_id, body, created_at) VALUES (1, 'backend', '{}', 'now');
             INSERT INTO lessons (id, curriculum_id, ord, phase_idx, module_idx, phase, module, title, objectives, status)
             VALUES (7, 1, 0, 0, 0, 'p', 'm', 't', '[]', 'todo');",
        )
        .unwrap();
        assert!(load(&conn, 7).unwrap().scene.is_none());
        save_scene(&conn, 7, "{\"elements\":[]}").unwrap();
        save_task(&conn, 7, "Draw a load balancer").unwrap();
        save_critique(&conn, 7, "Good start").unwrap();
        save_scene(&conn, 7, "{\"elements\":[1]}").unwrap();
        let s = load(&conn, 7).unwrap();
        assert_eq!(s.scene.as_deref(), Some("{\"elements\":[1]}"));
        assert_eq!(s.task.as_deref(), Some("Draw a load balancer"));
        assert_eq!(s.critique.as_deref(), Some("Good start"));
        save_task(&conn, 7, "New task").unwrap();
        assert!(load(&conn, 7).unwrap().critique.is_none(), "a new task clears the old critique");
    }
}
