//! Mock interviews: an AI interviewer asks one question at a time against a time budget,
//! then writes a scored debrief.

use anyhow::{bail, Result};
use chrono::{DateTime, Local};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::db::{self, Track};
use crate::llm::{ChatMsg, Provider};
use crate::tutor;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Behavioral,
    Technical,
    Dsa,
    SystemDesign,
}

impl Kind {
    pub fn parse(s: &str) -> Result<Kind> {
        Ok(match s {
            "behavioral" => Kind::Behavioral,
            "technical" => Kind::Technical,
            "dsa" => Kind::Dsa,
            "system_design" => Kind::SystemDesign,
            _ => bail!("unknown interview type {s}"),
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Behavioral => "behavioral",
            Kind::Technical => "technical",
            Kind::Dsa => "dsa",
            Kind::SystemDesign => "system_design",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Behavioral => "Behavioral",
            Kind::Technical => "Technical questions",
            Kind::Dsa => "Coding (DSA)",
            Kind::SystemDesign => "System design",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Interview {
    pub id: i64,
    pub kind: Kind,
    pub track_id: Option<String>,
    pub minutes: i64,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub debrief: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Turn {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub code: Option<String>,
    pub created_at: String,
}

pub fn create(conn: &Connection, kind: Kind, track_id: Option<&str>, minutes: i64) -> Result<i64> {
    if !(5..=90).contains(&minutes) {
        bail!("Interviews can be 5 to 90 minutes.");
    }
    conn.execute(
        "INSERT INTO interviews (kind, track_id, minutes, started_at) VALUES (?1, ?2, ?3, ?4)",
        params![kind.as_str(), track_id, minutes, db::now()],
    )?;
    Ok(conn.last_insert_rowid())
}

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Interview> {
    let kind: String = r.get(1)?;
    let debrief: Option<String> = r.get(6)?;
    Ok(Interview {
        id: r.get(0)?,
        kind: Kind::parse(&kind).unwrap_or(Kind::Technical),
        track_id: r.get(2)?,
        minutes: r.get(3)?,
        started_at: r.get(4)?,
        ended_at: r.get(5)?,
        debrief: debrief.and_then(|d| serde_json::from_str(&d).ok()),
    })
}

const COLS: &str = "id, kind, track_id, minutes, started_at, ended_at, debrief";

pub fn get(conn: &Connection, id: i64) -> Result<Interview> {
    Ok(conn.query_row(&format!("SELECT {COLS} FROM interviews WHERE id = ?1"), [id], from_row)?)
}

pub fn list(conn: &Connection) -> Result<Vec<Interview>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM interviews ORDER BY id DESC LIMIT 50"))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub fn turns(conn: &Connection, id: i64) -> Result<Vec<Turn>> {
    let mut stmt = conn.prepare(
        "SELECT id, role, content, code, created_at FROM interview_messages WHERE interview_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([id], |r| {
        Ok(Turn { id: r.get(0)?, role: r.get(1)?, content: r.get(2)?, code: r.get(3)?, created_at: r.get(4)? })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub fn system_prompt(kind: Kind, track: Option<&Track>, minutes: i64, known: &[String]) -> String {
    let base = format!(
        "You are a friendly but rigorous interviewer at a strong tech company, interviewing a university student \
(graduating 2028) for a software engineering internship or junior remote role. Interview length: {minutes} minutes.\n\
Rules:\n\
- Ask ONE question at a time, then stop and wait. Keep each turn under 80 words: it may be read aloud, so write the \
way you would speak. No tables, no headings, no emoji. Use code formatting only when quoting code.\n\
- Probe with follow-ups (\"why?\", \"what would break?\", \"how did you measure that?\") before moving on. \
Don't teach, hint heavily or reveal answers during the interview; a brief neutral acknowledgement is fine.\n\
- Each candidate message starts with a <time_left> note. Pace yourself; when 2 minutes or less remain, ask your last \
question or invite their questions, then close politely.\n\
- If the candidate's answer is empty or off-topic, say so kindly and re-ask.\n"
    );
    let specific = match kind {
        Kind::Behavioral => "Type: behavioral. Ask about projects, teamwork, conflict, failure, learning something hard, \
ownership. Push for STAR structure: the situation, their own task, the specific actions THEY took, and the measurable result. \
Ask for specifics when answers are vague."
            .to_string(),
        Kind::Technical => format!(
            "Type: technical concepts for {}. Syllabus: {}. Mix 'explain X' questions with 'how would you…' scenarios and \
'what goes wrong when…' questions. Start easy; go deeper when answers are strong, step back when they struggle.",
            track.map(|t| t.name).unwrap_or("backend engineering"),
            track.map(|t| t.outline).unwrap_or(""),
        ),
        Kind::Dsa => "Type: coding interview. Pose ONE problem of easy-medium difficulty (arrays, strings, hash maps, \
two pointers, stacks, trees or BFS). State it clearly with one example. Let them ask clarifying questions, ask them to explain \
an approach before coding, then to code it (their code arrives in <code> blocks), then ask for time and space complexity, edge \
cases and a dry run. Point out bugs only by asking about a failing input."
            .to_string(),
        Kind::SystemDesign => "Type: system design, scaled for an intern. Pick one: URL shortener, rate limiter, \
notification service, file upload service, or a chat backend. Guide through: requirements and scale, API, data model, \
high-level components, then one deep dive (caching, database choice, queues or scaling). Ask about trade-offs."
            .to_string(),
    };
    let known_s = if known.is_empty() { String::new() } else { format!("\nThe candidate says they know: {}.", known.join(", ")) };
    format!("{base}{specific}{known_s}")
}

pub fn time_left_note(started: &str, minutes: i64, now: DateTime<Local>) -> String {
    let start = DateTime::parse_from_rfc3339(started).map(|t| t.with_timezone(&Local)).unwrap_or(now);
    let left = minutes * 60 - (now - start).num_seconds();
    if left <= 0 {
        "<time_left>Time is up. Close the interview now.</time_left>\n".into()
    } else {
        format!("<time_left>{} min {} s left of {minutes} min</time_left>\n", left / 60, left % 60)
    }
}

pub const START: &str = "(The candidate has joined. Greet them in one sentence and ask your first question.)";

/// Build the message list for the next interviewer turn and store the candidate's message.
pub fn prepare_turn(conn: &Connection, iv: &Interview, text: &str, code: Option<&str>) -> Result<Vec<ChatMsg>> {
    if iv.ended_at.is_some() {
        bail!("This interview has ended.");
    }
    let mut msgs: Vec<ChatMsg> = turns(conn, iv.id)?
        .into_iter()
        .map(|t| {
            let mut c = t.content;
            if let Some(code) = t.code.filter(|c| !c.trim().is_empty()) {
                c.push_str(&format!("\n<code>\n{code}\n</code>"));
            }
            ChatMsg::text(&t.role, c)
        })
        .collect();
    let first = msgs.is_empty();
    let shown = if first { START.to_string() } else { text.trim().to_string() };
    if !first && shown.is_empty() && code.map(str::trim).unwrap_or("").is_empty() {
        bail!("Say or type an answer first.");
    }
    let note = time_left_note(&iv.started_at, iv.minutes, Local::now());
    let mut content = format!("{note}{shown}");
    if let Some(c) = code.filter(|c| !c.trim().is_empty()) {
        content.push_str(&format!("\n<code>\n{c}\n</code>"));
    }
    conn.execute(
        "INSERT INTO interview_messages (interview_id, role, content, code, created_at) VALUES (?1, 'user', ?2, ?3, ?4)",
        params![iv.id, shown, code.filter(|c| !c.trim().is_empty()), db::now()],
    )?;
    msgs.push(ChatMsg::text("user", content));
    Ok(msgs)
}

pub fn save_reply(conn: &Connection, id: i64, reply: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO interview_messages (interview_id, role, content, created_at) VALUES (?1, 'assistant', ?2, ?3)",
        params![id, reply, db::now()],
    )?;
    Ok(conn.last_insert_rowid())
}

fn debrief_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "overall": {"type": "integer"},
            "verdict": {"type": "string"},
            "summary": {"type": "string"},
            "scores": {"type": "array", "items": {
                "type": "object",
                "properties": {"skill": {"type": "string"}, "score": {"type": "integer"}, "note": {"type": "string"}},
                "required": ["skill", "score", "note"], "additionalProperties": false
            }},
            "strengths": {"type": "array", "items": {"type": "string"}},
            "improvements": {"type": "array", "items": {"type": "string"}},
            "better_answer": {"type": "object",
                "properties": {"question": {"type": "string"}, "answer": {"type": "string"}},
                "required": ["question", "answer"], "additionalProperties": false}
        },
        "required": ["overall", "verdict", "summary", "scores", "strengths", "improvements", "better_answer"],
        "additionalProperties": false
    })
}

/// Score the interview, store the debrief and close it.
pub async fn debrief(conn_lock: &std::sync::Mutex<Connection>, provider: &Provider, id: i64) -> Result<(Value, crate::llm::Usage)> {
    let (iv, ts) = {
        let conn = conn_lock.lock().unwrap();
        (get(&conn, id)?, turns(&conn, id)?)
    };
    let answered = ts.iter().filter(|t| t.role == "user" && t.content != START && !t.content.is_empty()).count();
    if answered == 0 {
        bail!("Answer at least one question before ending, or there's nothing to score.");
    }
    let transcript: String = ts
        .iter()
        .filter(|t| t.content != START)
        .map(|t| {
            let who = if t.role == "user" { "CANDIDATE" } else { "INTERVIEWER" };
            let code = t.code.as_ref().map(|c| format!("\n[code]\n{c}\n[/code]")).unwrap_or_default();
            format!("{who}: {}{code}", t.content)
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let skills = match iv.kind {
        Kind::Behavioral => "Structure (STAR), Specificity and ownership, Impact and results, Communication",
        Kind::Technical => "Technical accuracy, Depth of understanding, Reasoning about trade-offs, Communication",
        Kind::Dsa => "Problem understanding, Approach and algorithm, Code correctness, Complexity analysis, Communication",
        Kind::SystemDesign => "Requirements gathering, High-level design, Data and API design, Trade-offs and scaling, Communication",
    };
    let system = "You are a hiring panel lead writing honest, useful interview feedback for a student. Score each skill \
1-5 (1 = not shown, 3 = acceptable for an intern, 5 = excellent) and justify each in one sentence citing what they said. \
`overall` is 1-5. `verdict` is one of: Strong hire, Hire, Lean hire, Lean no hire, No hire (for an internship bar). \
`summary` is two sentences. Give 2-3 strengths and the 3 most valuable improvements as concrete actions. \
`better_answer` rewrites their weakest answer as a strong one (under 150 words). Judge only what is in the transcript; \
if the interview was short, say so in the summary. No emoji.";
    let user = format!("Interview type: {}\nSkills to score: {skills}\n\nTranscript:\n{transcript}", iv.kind.label());
    let (mut v, usage) = provider.complete_json(system, &user, debrief_schema()).await?;
    // Clamp scores into range: models occasionally drift.
    if let Some(o) = v["overall"].as_i64() {
        v["overall"] = json!(o.clamp(1, 5));
    }
    if let Some(arr) = v["scores"].as_array_mut() {
        for s in arr {
            let n = s["score"].as_i64().unwrap_or(3).clamp(1, 5);
            s["score"] = json!(n);
            s["skill"] = json!(tutor::loose_text(&s["skill"]));
            s["note"] = json!(tutor::loose_text(&s["note"]));
        }
    }
    let conn = conn_lock.lock().unwrap();
    db::record_usage(&conn, &usage)?;
    conn.execute(
        "UPDATE interviews SET debrief = ?1, ended_at = ?2 WHERE id = ?3",
        params![v.to_string(), db::now(), id],
    )?;
    Ok((v, usage))
}

pub fn known_topics(conn: &Connection, track_id: Option<&str>) -> Vec<String> {
    track_id
        .and_then(|t| {
            conn.query_row("SELECT known FROM placements WHERE track_id = ?1", [t], |r| r.get::<_, String>(0))
                .optional()
                .ok()
                .flatten()
        })
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interview_turns_carry_time_and_code() {
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        let id = create(&conn, Kind::Dsa, None, 20).unwrap();
        let iv = get(&conn, id).unwrap();
        let first = prepare_turn(&conn, &iv, "", None).unwrap();
        assert_eq!(first.len(), 1);
        assert!(first[0].content.starts_with("<time_left>19 min") || first[0].content.starts_with("<time_left>20 min"));
        save_reply(&conn, id, "Hi! Given an array, return the two indices that sum to target.").unwrap();
        let second = prepare_turn(&conn, &iv, "Use a hash map.", Some("def two_sum(a, t): ...")).unwrap();
        assert_eq!(second.len(), 3);
        assert!(second[2].content.contains("<code>"));
        assert!(prepare_turn(&conn, &iv, "  ", None).is_err(), "empty answers are rejected");
        assert_eq!(turns(&conn, id).unwrap().len(), 3);
    }

    #[test]
    fn time_note_counts_down() {
        let start = Local::now() - chrono::Duration::minutes(19);
        let note = time_left_note(&start.to_rfc3339(), 20, Local::now());
        assert!(note.starts_with("<time_left>0 min") || note.starts_with("<time_left>1 min"), "{note}");
        let over = time_left_note(&(Local::now() - chrono::Duration::minutes(30)).to_rfc3339(), 20, Local::now());
        assert!(over.contains("Time is up"));
    }
}
