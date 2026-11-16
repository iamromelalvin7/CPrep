//! Teaching logic: placement quiz, curriculum design, lesson chat with retrieval and citations.

use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::config::Config;
use crate::db::{self, Track};
use crate::llm::{self, ChatMsg, Provider, Task, Usage};
use crate::rag::{self, Source};


// ---------- Lenient model-output parsing ----------
// Models sometimes return {"text": "..."} or a number where a plain string was asked for.

/// Flatten any JSON value into readable text.
pub fn loose_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(a) => a.iter().map(loose_text).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("; "),
        Value::Object(o) => {
            for key in ["text", "title", "objective", "description", "name", "value", "content"] {
                if let Some(x) = o.get(key) {
                    return loose_text(x);
                }
            }
            o.values().map(loose_text).find(|s| !s.is_empty()).unwrap_or_default()
        }
        Value::Null => String::new(),
    }
}

fn de_text<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    Ok(loose_text(&Value::deserialize(d)?))
}

fn de_texts<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Array(a) => a.iter().map(loose_text).filter(|s| !s.is_empty()).collect(),
        Value::Null => vec![],
        other => vec![loose_text(&other)],
    })
}

fn de_bool<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<bool, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Bool(b) => b,
        Value::String(s) => s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("yes"),
        Value::Number(n) => n.as_i64() == Some(1),
        _ => false,
    })
}

fn de_index<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<usize, D::Error> {
    let v = Value::deserialize(d)?;
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        .map(|n| n as usize)
        .ok_or_else(|| serde::de::Error::custom("answer is not an index"))
}

// ---------- Placement ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Question {
    #[serde(deserialize_with = "de_text")]
    pub topic: String,
    #[serde(deserialize_with = "de_text")]
    pub prompt: String,
    #[serde(deserialize_with = "de_texts")]
    pub options: Vec<String>,
    #[serde(deserialize_with = "de_index")]
    pub answer: usize,
    #[serde(default, deserialize_with = "de_text")]
    pub explanation: String,
}

/// What the UI sees: no answers until submitted.
#[derive(Clone, Debug, Serialize)]
pub struct PublicQuestion {
    pub topic: String,
    pub prompt: String,
    pub options: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopicScore {
    pub topic: String,
    pub correct: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Review {
    pub prompt: String,
    pub chosen: Option<usize>,
    pub answer: usize,
    pub options: Vec<String>,
    pub explanation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlacementResult {
    pub correct: usize,
    pub total: usize,
    pub topics: Vec<TopicScore>,
    /// Topics answered fully correctly: candidates to skip.
    pub strong: Vec<String>,
    pub weak: Vec<String>,
    pub review: Vec<Review>,
}

fn quiz_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "questions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "topic": {"type": "string"},
                        "prompt": {"type": "string"},
                        "options": {"type": "array", "items": {"type": "string"}},
                        "answer": {"type": "integer"},
                        "explanation": {"type": "string"}
                    },
                    "required": ["topic", "prompt", "options", "answer", "explanation"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["questions"],
        "additionalProperties": false
    })
}

/// Drop malformed questions rather than showing the learner something broken.
pub fn validate_questions(v: &Value) -> Vec<Question> {
    let Some(arr) = v["questions"].as_array() else { return vec![] };
    arr.iter()
        .filter_map(|q| serde_json::from_value::<Question>(q.clone()).ok())
        .filter(|q| {
            q.options.len() >= 2
                && q.options.len() <= 6
                && q.answer < q.options.len()
                && !q.prompt.trim().is_empty()
                && q.options.iter().all(|o| !o.trim().is_empty())
        })
        .collect()
}

pub async fn generate_quiz(
    provider: &Provider,
    track: &Track,
    known: &[String],
    note: &str,
) -> Result<(Vec<Question>, Usage)> {
    let system = "You write short, fair placement quizzes for a self-taught computer science student. \
        Every question is multiple choice with exactly 4 options and one correct answer. \
        `answer` is the 0-based index of the correct option. Vary which index is correct. \
        Questions test real understanding (reading code, predicting output, choosing a design), not trivia. \
        Keep each prompt under 60 words; put code in Markdown backticks. Explanations are one or two sentences.";
    let known_s = if known.is_empty() { "nothing specific".to_string() } else { known.join(", ") };
    let user = format!(
        "Track: {name}\nTrack syllabus: {outline}\n\nThe learner says they already know: {known_s}.\nTheir note: {note}\n\n\
         Write 8 questions. Spend about 5 on the topics they claim to know (to verify the claim, so they can skip it) \
         and about 3 on early topics of the syllabus. Set `topic` to a short syllabus topic name (2-4 words) and reuse \
         the same name for questions on the same topic.",
        name = track.name,
        outline = track.outline,
        note = if note.trim().is_empty() { "-" } else { note },
    );
    let (v, usage) = provider.complete_json(system, &user, quiz_schema()).await?;
    let qs = validate_questions(&v);
    if qs.len() < 3 {
        bail!("The model produced an unusable quiz. Try again (small local models sometimes stumble).");
    }
    Ok((qs, usage))
}

pub fn grade(questions: &[Question], answers: &[Option<usize>]) -> PlacementResult {
    let mut by: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut review = Vec::new();
    let mut correct = 0;
    for (i, q) in questions.iter().enumerate() {
        let chosen = answers.get(i).copied().flatten();
        let ok = chosen == Some(q.answer);
        let e = by.entry(q.topic.trim().to_string()).or_default();
        e.1 += 1;
        if ok {
            e.0 += 1;
            correct += 1;
        }
        review.push(Review {
            prompt: q.prompt.clone(),
            chosen,
            answer: q.answer,
            options: q.options.clone(),
            explanation: q.explanation.clone(),
        });
    }
    let topics: Vec<TopicScore> =
        by.into_iter().map(|(topic, (c, t))| TopicScore { topic, correct: c, total: t }).collect();
    PlacementResult {
        correct,
        total: questions.len(),
        strong: topics.iter().filter(|t| t.correct == t.total).map(|t| t.topic.clone()).collect(),
        weak: topics.iter().filter(|t| t.correct * 2 < t.total || t.correct == 0).map(|t| t.topic.clone()).collect(),
        topics,
        review,
    }
}

// ---------- Curriculum ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CLesson {
    #[serde(deserialize_with = "de_text")]
    pub title: String,
    #[serde(default, deserialize_with = "de_texts")]
    pub objectives: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CModule {
    #[serde(deserialize_with = "de_text")]
    pub title: String,
    #[serde(default, deserialize_with = "de_bool")]
    pub tested_out: bool,
    pub lessons: Vec<CLesson>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CPhase {
    #[serde(deserialize_with = "de_text")]
    pub title: String,
    #[serde(default, deserialize_with = "de_text")]
    pub goal: String,
    pub modules: Vec<CModule>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Curriculum {
    #[serde(deserialize_with = "de_text")]
    pub title: String,
    #[serde(default, deserialize_with = "de_text")]
    pub summary: String,
    pub phases: Vec<CPhase>,
}

fn curriculum_schema() -> Value {
    let lesson = json!({
        "type": "object",
        "properties": {"title": {"type": "string"}, "objectives": {"type": "array", "items": {"type": "string"}}},
        "required": ["title", "objectives"], "additionalProperties": false
    });
    let module = json!({
        "type": "object",
        "properties": {"title": {"type": "string"}, "tested_out": {"type": "boolean"}, "lessons": {"type": "array", "items": lesson}},
        "required": ["title", "tested_out", "lessons"], "additionalProperties": false
    });
    let phase = json!({
        "type": "object",
        "properties": {"title": {"type": "string"}, "goal": {"type": "string"}, "modules": {"type": "array", "items": module}},
        "required": ["title", "goal", "modules"], "additionalProperties": false
    });
    json!({
        "type": "object",
        "properties": {"title": {"type": "string"}, "summary": {"type": "string"}, "phases": {"type": "array", "items": phase}},
        "required": ["title", "summary", "phases"], "additionalProperties": false
    })
}

pub fn validate_curriculum(v: Value) -> Result<Curriculum> {
    let raw = v.to_string();
    let mut c: Curriculum = serde_json::from_value(v)
        .map_err(|e| anyhow!("curriculum shape was wrong: {e}. Model output began: {}", raw.chars().take(300).collect::<String>()))?;
    for p in &mut c.phases {
        for m in &mut p.modules {
            m.lessons.retain(|l| !l.title.trim().is_empty());
        }
        p.modules.retain(|m| !m.lessons.is_empty());
    }
    c.phases.retain(|p| !p.modules.is_empty());
    if c.phases.is_empty() {
        bail!("The model produced an empty curriculum. Try again.");
    }
    Ok(c)
}

pub async fn design_curriculum(
    provider: &Provider,
    track: &Track,
    placement: Option<&PlacementResult>,
    known: &[String],
    library: &[String],
) -> Result<(Curriculum, Usage)> {
    let system = "You are a senior engineer designing a from-the-ground-up, project-driven course for one \
        student preparing for internships and remote software roles. Order topics so each builds on the last. \
        Lessons are 30-60 minute sessions. Each lesson's `objectives` is its ordered list of 3-6 topics, each a short \
        phrase naming what gets taught (e.g. \"Primitive types and operators\", \"Conditional statements\", \"Loops\"). \
        Mark a module `tested_out: true` only when the placement results show the learner already knows it.";
    let placement_s = match placement {
        Some(p) => format!(
            "Placement: {}/{} correct. Strong (all correct): {}. Weak: {}.",
            p.correct,
            p.total,
            if p.strong.is_empty() { "none".into() } else { p.strong.join(", ") },
            if p.weak.is_empty() { "none".into() } else { p.weak.join(", ") },
        ),
        None => "No placement quiz taken.".into(),
    };
    let lib = if library.is_empty() {
        "(no files connected)".to_string()
    } else {
        library.iter().take(80).map(|t| format!("- {t}")).collect::<Vec<_>>().join("\n")
    };
    let user = format!(
        "Track: {}\nSyllabus to cover: {}\n\nLearner claims to know: {}\n{placement_s}\n\n\
         The learner's own study files (prefer topics these cover, and teach in the same frameworks):\n{lib}\n\n\
         Design the course: 3-5 phases, each with 2-3 modules, each module with 2-3 lessons.",
        track.name,
        track.outline,
        if known.is_empty() { "nothing specific".into() } else { known.join(", ") },
    );
    let (v, usage) = provider.complete_json(system, &user, curriculum_schema()).await?;
    Ok((validate_curriculum(v)?, usage))
}

/// Store a curriculum and flatten it into ordered lessons. Returns the curriculum id.
pub fn save_curriculum(conn: &Connection, track_id: &str, c: &Curriculum) -> Result<i64> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO curricula (track_id, body, created_at) VALUES (?1, ?2, ?3)",
        params![track_id, serde_json::to_string(c)?, db::now()],
    )?;
    let cid = tx.last_insert_rowid();
    let mut ord = 0;
    for (pi, p) in c.phases.iter().enumerate() {
        for (mi, m) in p.modules.iter().enumerate() {
            for l in &m.lessons {
                tx.execute(
                    "INSERT INTO lessons (curriculum_id, ord, phase_idx, module_idx, phase, module, title, objectives, status)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        cid,
                        ord,
                        pi as i64,
                        mi as i64,
                        p.title,
                        m.title,
                        l.title,
                        serde_json::to_string(&l.objectives)?,
                        if m.tested_out { "skipped" } else { "todo" }
                    ],
                )?;
                ord += 1;
            }
        }
    }
    tx.commit()?;
    Ok(cid)
}

// ---------- Lessons ----------

#[derive(Clone, Debug, Serialize)]
pub struct Lesson {
    pub id: i64,
    pub curriculum_id: i64,
    pub ord: i64,
    pub phase_idx: i64,
    pub module_idx: i64,
    pub phase: String,
    pub module: String,
    pub title: String,
    /// The lesson's topics, taught in order.
    pub objectives: Vec<String>,
    pub status: String,
    /// 0-based index of the topic the tutor is on.
    pub topic_idx: i64,
}

pub fn lesson_from_row(r: &rusqlite::Row) -> rusqlite::Result<Lesson> {
    let obj: String = r.get(7)?;
    Ok(Lesson {
        id: r.get(0)?,
        curriculum_id: r.get(1)?,
        ord: r.get(2)?,
        phase_idx: r.get(3)?,
        module_idx: r.get(4)?,
        phase: r.get(5)?,
        module: r.get(6)?,
        objectives: serde_json::from_str(&obj).unwrap_or_default(),
        title: r.get(8)?,
        status: r.get(9)?,
        topic_idx: r.get(10)?,
    })
}

pub const LESSON_COLS: &str =
    "id, curriculum_id, ord, phase_idx, module_idx, phase, module, objectives, title, status, topic_idx";

pub fn get_lesson(conn: &Connection, id: i64) -> Result<Lesson> {
    Ok(conn.query_row(&format!("SELECT {LESSON_COLS} FROM lessons WHERE id = ?1"), [id], lesson_from_row)?)
}

#[derive(Clone, Debug, Serialize)]
pub struct Message {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub sources: Vec<Source>,
    pub created_at: String,
    /// For tutor replies: the 0-based topic this reply teaches.
    pub topic_idx: Option<i64>,
}

pub fn messages(conn: &Connection, lesson_id: i64) -> Result<Vec<Message>> {
    let mut stmt = conn.prepare(
        "SELECT id, role, content, sources, created_at, topic_idx FROM messages WHERE lesson_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([lesson_id], |r| {
        let s: Option<String> = r.get(3)?;
        Ok(Message {
            id: r.get(0)?,
            role: r.get(1)?,
            content: r.get(2)?,
            sources: s.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default(),
            created_at: r.get(4)?,
            topic_idx: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Stable per lesson, so Claude can cache it.
pub fn lesson_system_prompt(track: &Track, lesson: &Lesson, placement: Option<&PlacementResult>) -> String {
    let topics = lesson.objectives.iter().enumerate().map(|(i, o)| format!("{}. {o}", i + 1)).collect::<Vec<_>>().join("\n");
    let learner = match placement {
        Some(p) => format!(
            "Placement quiz: {}/{}. Strong: {}. Weak: {}.",
            p.correct,
            p.total,
            if p.strong.is_empty() { "none".into() } else { p.strong.join(", ") },
            if p.weak.is_empty() { "none".into() } else { p.weak.join(", ") }
        ),
        None => "No placement data yet.".into(),
    };
    format!(
        "You are CPrep, a patient, rigorous tutor for a university student (graduating 2028) preparing for \
software engineering internships and remote roles. Track: {track}.\n\n\
Current lesson: \"{title}\" (phase \"{phase}\", module \"{module}\").\nTopics, taught in this order:\n{topics}\n\n\
Learner profile: {learner}\n\n\
How to teach:\n\
- Teach the topics in order, one idea at a time, from the ground up, with a concrete example (Python unless the lesson says otherwise).\n\
- Begin EVERY reply with a marker on its own first line giving the number of the topic that reply is about, exactly like: [[topic:2]]\n\
- Move to the next topic only when the learner has shown they understand the current one. Each learner message \
starts with a <lesson_state> note saying which topic they are on.\n\
- Keep each reply under about 250 words. End with ONE check question or a tiny exercise, then wait.\n\
- When the learner answers, say clearly whether it is right, fix misconceptions, then move on.\n\
- Each learner message may include <sources> excerpts from their own study files, numbered [S1], [S2]... \
Prefer them, and cite them inline like [S2] when you use them. If they don't cover something, teach from \
general knowledge and don't invent citations.\n\
- When every topic is covered, summarise in 3 bullets and say the lesson can be marked complete.\n\
- Use Markdown. Use fenced code blocks with a language tag. Do not use emoji anywhere.",
        track = track.name,
        title = lesson.title,
        phase = lesson.phase,
        module = lesson.module,
    )
}

static TOPIC_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"\[\[\s*topic\s*:\s*(\d+)\s*\]\]\s*").unwrap());

/// Find the tutor's `[[topic:N]]` marker (1-based); return the 0-based topic and the reply without markers.
pub fn parse_topic(reply: &str, topics: usize) -> (Option<i64>, String) {
    let topic = TOPIC_RE
        .captures(reply)
        .and_then(|c| c[1].parse::<i64>().ok())
        .filter(|_| topics > 0)
        .map(|n| (n - 1).clamp(0, topics as i64 - 1));
    (topic, TOPIC_RE.replace_all(reply, "").trim().to_string())
}

/// Tells the tutor where the learner is. Goes in the user turn so the system prompt stays cacheable.
pub fn lesson_state(lesson: &Lesson) -> String {
    let n = lesson.objectives.len();
    if n == 0 {
        return String::new();
    }
    let i = (lesson.topic_idx.max(0) as usize).min(n - 1);
    format!("<lesson_state>Topic {} of {n}: {}</lesson_state>\n", i + 1, lesson.objectives[i])
}

pub fn format_sources(sources: &[Source]) -> String {
    if sources.is_empty() {
        return String::new();
    }
    let mut s = String::from("<sources>\n");
    for (i, src) in sources.iter().enumerate() {
        let page = src.page.map(|p| format!(", p. {p}")).unwrap_or_default();
        s.push_str(&format!("[S{}] {}{}\n{}\n\n", i + 1, src.title, page, src.text));
    }
    s.push_str("</sources>\n\n");
    s
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ChatEvent {
    Sources { sources: Vec<Source> },
    Delta { text: String },
    Done { message_id: i64, provider: String, cost_usd: f64, topic_idx: i64 },
    Error { message: String },
}

pub const START_PROMPT: &str = "Start the lesson. Briefly say what we'll cover, then teach the first idea.";

/// One tutoring turn: retrieve, build the prompt, stream the reply, save both sides.
#[allow(clippy::too_many_arguments)]
pub async fn chat_turn(
    cfg: &Config,
    db: &Mutex<Connection>,
    provider: &Provider,
    track: &Track,
    lesson_id: i64,
    user_text: &str,
    mut emit: impl FnMut(ChatEvent),
) -> Result<i64> {
    let (lesson, placement, history) = {
        let conn = db.lock().unwrap();
        let lesson = get_lesson(&conn, lesson_id)?;
        let placement = load_placement_result(&conn, track.id)?;
        let mut stmt =
            conn.prepare("SELECT role, content, llm_content FROM messages WHERE lesson_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map([lesson_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        let history: Vec<(String, String, String)> = rows.collect::<std::result::Result<_, _>>()?;
        (lesson, placement, history)
    };

    let is_start = user_text.trim().is_empty();
    let query = if is_start {
        format!("{} {}", lesson.title, lesson.objectives.join(" "))
    } else {
        format!("{} {}", lesson.title, user_text)
    };
    let sources = rag::search(cfg, db, track.id, &query, 5).await.unwrap_or_default();
    emit(ChatEvent::Sources { sources: sources.clone() });

    let shown = if is_start { "" } else { user_text };
    let asked = if is_start { START_PROMPT } else { user_text };
    let llm_user = format!("{}{}{}", lesson_state(&lesson), format_sources(&sources), asked);

    // Claude gets the exact history (cache-friendly). Small local models get a trimmed window
    // where older turns drop their source excerpts, to fit an 8K context.
    let is_claude = matches!(provider, Provider::Claude { .. });
    let mut msgs: Vec<ChatMsg> = Vec::new();
    let window = if is_claude { history.len() } else { 8.min(history.len()) };
    for (role, content, llm_content) in &history[history.len() - window..] {
        let c = if is_claude {
            llm_content.clone()
        } else if role == "user" && content.is_empty() {
            START_PROMPT.to_string()
        } else {
            content.clone()
        };
        msgs.push(ChatMsg { role: role.clone(), content: c });
    }
    msgs.push(ChatMsg { role: "user".into(), content: llm_user.clone() });

    let system = lesson_system_prompt(track, &lesson, placement.as_ref());
    let (reply, usage) = provider
        .stream_chat(&system, &msgs, |t| emit(ChatEvent::Delta { text: t.to_string() }))
        .await?;
    let raw_reply = reply.trim().to_string();
    let (topic, reply) = parse_topic(&raw_reply, lesson.objectives.len());
    if reply.is_empty() {
        bail!("The tutor returned an empty reply. Try again.");
    }
    let topic_idx = topic.unwrap_or(lesson.topic_idx);

    let conn = db.lock().unwrap();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO messages (lesson_id, role, content, llm_content, created_at) VALUES (?1, 'user', ?2, ?3, ?4)",
        params![lesson_id, shown, llm_user, db::now()],
    )?;
    tx.execute(
        "INSERT INTO messages (lesson_id, role, content, llm_content, sources, created_at, topic_idx)
         VALUES (?1, 'assistant', ?2, ?3, ?4, ?5, ?6)",
        params![lesson_id, reply, raw_reply, serde_json::to_string(&sources)?, db::now(), topic_idx],
    )?;
    let mid = tx.last_insert_rowid();
    // Progress only moves forward; revisiting an earlier topic doesn't undo it.
    tx.execute("UPDATE lessons SET topic_idx = MAX(topic_idx, ?1) WHERE id = ?2", params![topic_idx, lesson_id])?;
    tx.execute("UPDATE lessons SET status = 'in_progress' WHERE id = ?1 AND status IN ('todo','skipped')", [lesson_id])?;
    db::record_usage(&tx, &usage)?;
    tx.commit()?;
    emit(ChatEvent::Done { message_id: mid, provider: provider.label_for(&usage), cost_usd: usage.cost_usd(), topic_idx });
    Ok(mid)
}

pub fn load_placement_result(conn: &Connection, track_id: &str) -> Result<Option<PlacementResult>> {
    let r: Option<Option<String>> = conn
        .query_row("SELECT result FROM placements WHERE track_id = ?1", [track_id], |r| r.get(0))
        .optional()?;
    Ok(r.flatten().and_then(|s| serde_json::from_str(&s).ok()))
}

pub fn route_for(cfg: &Config, live: bool, task: Task) -> Result<Provider> {
    llm::route(cfg, live, task)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(topic: &str, answer: usize) -> Question {
        Question {
            topic: topic.into(),
            prompt: "p".into(),
            options: vec!["a".into(), "b".into(), "c".into(), "d".into()],
            answer,
            explanation: "e".into(),
        }
    }

    #[test]
    fn grading_finds_strong_and_weak_topics() {
        let qs = vec![q("Flask", 0), q("Flask", 1), q("SQL", 2), q("SQL", 3)];
        let r = grade(&qs, &[Some(0), Some(1), Some(0), None]);
        assert_eq!(r.correct, 2);
        assert_eq!(r.strong, vec!["Flask"]);
        assert_eq!(r.weak, vec!["SQL"]);
    }

    #[test]
    fn rejects_malformed_questions() {
        let v = json!({"questions": [
            {"topic": "t", "prompt": "ok", "options": ["a","b","c","d"], "answer": 1, "explanation": "x"},
            {"topic": "t", "prompt": "bad index", "options": ["a","b"], "answer": 5, "explanation": "x"},
            {"topic": "t", "prompt": "", "options": ["a","b"], "answer": 0, "explanation": "x"},
            {"oops": true}
        ]});
        assert_eq!(validate_questions(&v).len(), 1);
    }

    #[test]
    fn curriculum_flattens_and_respects_tested_out() {
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        let c = validate_curriculum(json!({
            "title": "Backend", "summary": "s",
            "phases": [{"title": "P1", "goal": "g", "modules": [
                {"title": "Python", "tested_out": true, "lessons": [{"title": "Syntax", "objectives": ["a"]}]},
                {"title": "HTTP", "tested_out": false, "lessons": [{"title": "Requests", "objectives": ["b"]}, {"title": "", "objectives": []}]}
            ]}, {"title": "Empty", "goal": "g", "modules": []}]
        }))
        .unwrap();
        assert_eq!(c.phases.len(), 1);
        let cid = save_curriculum(&conn, "backend", &c).unwrap();
        let statuses: Vec<(String, String)> = conn
            .prepare("SELECT title, status FROM lessons WHERE curriculum_id = ?1 ORDER BY ord")
            .unwrap()
            .query_map([cid], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(statuses, vec![("Syntax".into(), "skipped".into()), ("Requests".into(), "todo".into())]);
    }

    #[test]
    fn tolerates_objects_where_strings_expected() {
        let c = validate_curriculum(json!({
            "title": {"text": "Backend"}, "summary": "s",
            "phases": [{"title": "P1", "goal": {"description": "g"}, "modules": [
                {"title": "HTTP", "tested_out": "false", "lessons": [
                    {"title": "Requests", "objectives": [{"objective": "Read a request"}, "Send a response", 3]}
                ]}
            ]}]
        }))
        .unwrap();
        assert_eq!(c.title, "Backend");
        assert_eq!(c.phases[0].goal, "g");
        assert_eq!(c.phases[0].modules[0].lessons[0].objectives, vec!["Read a request", "Send a response", "3"]);
        let qs = validate_questions(&json!({"questions": [
            {"topic": "t", "prompt": "p", "options": [{"text": "a"}, "b"], "answer": "1", "explanation": "x"}
        ]}));
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].options[0], "a");
    }

    #[test]
    fn topic_marker_is_parsed_and_stripped() {
        let (t, text) = parse_topic("[[topic:2]]\nLoops repeat work.", 4);
        assert_eq!(t, Some(1));
        assert_eq!(text, "Loops repeat work.");
        assert_eq!(parse_topic("[[ topic: 9 ]] x", 3).0, Some(2), "clamped to the last topic");
        assert_eq!(parse_topic("no marker", 3).0, None);
    }

    #[test]
    fn sources_are_numbered() {
        let s = format_sources(&[Source {
            chunk_id: 1,
            title: "flask.pdf".into(),
            path: "x".into(),
            page: Some(3),
            text: "routes".into(),
            score: 0.9,
        }]);
        assert!(s.contains("[S1] flask.pdf, p. 3"));
    }
}
