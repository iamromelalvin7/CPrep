//! Code lab: AI-written exercises (as data), local grading, AI feedback, and study notes.

use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::config::Config;
use crate::db::{self, Track};
use crate::llm::{ChatMsg, Provider};
use crate::runner::{Grade, Kind, Language, TestCase};
use crate::{rag, tutor};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Exercise {
    pub title: String,
    pub prompt: String,
    pub language: Language,
    pub kind: Kind,
    #[serde(default)]
    pub function_name: String,
    #[serde(default)]
    pub starter_code: String,
    #[serde(default)]
    pub setup_sql: String,
    #[serde(default)]
    pub ordered: bool,
    pub tests: Vec<TestCase>,
}

/// What the UI sees: hidden tests stay hidden.
#[derive(Clone, Debug, Serialize)]
pub struct ExerciseView {
    pub id: i64,
    pub lesson_id: i64,
    pub topic_idx: i64,
    pub title: String,
    pub prompt: String,
    pub language: Language,
    pub kind: Kind,
    pub function_name: String,
    pub starter_code: String,
    pub setup_sql: String,
    pub draft: Option<String>,
    pub examples: Vec<TestCase>,
    pub hidden_count: usize,
    pub best: Option<(i64, i64)>,
    pub attempts: i64,
}

// ---------- Generation ----------

fn exercise_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": {"type": "string"},
            "prompt": {"type": "string"},
            "language": {"type": "string", "enum": ["python", "javascript", "sql", "java"]},
            "kind": {"type": "string", "enum": ["function", "output", "query"]},
            "function_name": {"type": "string"},
            "starter_code": {"type": "string"},
            "setup_sql": {"type": "string"},
            "ordered": {"type": "boolean"},
            "tests": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "args_json": {"type": "string"},
                        "expected_json": {"type": "string"},
                        "stdin": {"type": "string"},
                        "expected_output": {"type": "string"},
                        "hidden": {"type": "boolean"}
                    },
                    "required": ["args_json", "expected_json", "stdin", "expected_output", "hidden"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["title", "prompt", "language", "kind", "function_name", "starter_code", "setup_sql", "ordered", "tests"],
        "additionalProperties": false
    })
}

/// Turn the model's JSON into a checked Exercise. Test values arrive as JSON strings so strict
/// structured-output modes can describe them; they are parsed here and never executed.
pub fn parse_exercise(v: &Value) -> Result<Exercise> {
    let text = |k: &str| tutor::loose_text(&v[k]);
    let language = Language::parse(&text("language")).unwrap_or(Language::Python);
    let mut kind = match text("kind").to_lowercase().as_str() {
        "output" => Kind::Output,
        "query" => Kind::Query,
        _ => Kind::Function,
    };
    if language == Language::Sql {
        kind = Kind::Query;
    } else if language == Language::Java || kind == Kind::Query {
        kind = Kind::Output;
    }
    let function_name = text("function_name");
    if kind == Kind::Function && !regex::Regex::new(r"^[A-Za-z_$][\w$]*$").unwrap().is_match(&function_name) {
        bail!("exercise has no valid function name");
    }
    let parse_json = |s: &Value| -> Option<Value> {
        match s {
            Value::String(s) if s.trim().is_empty() => Some(Value::Null),
            Value::String(s) => serde_json::from_str(s).ok().or_else(|| Some(Value::String(s.clone()))),
            other => Some(other.clone()),
        }
    };
    let tests: Vec<TestCase> = v["tests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let args = match parse_json(&t["args_json"])? {
                Value::Array(a) => a,
                Value::Null => vec![],
                single => vec![single],
            };
            let tc = TestCase {
                args,
                expected: parse_json(&t["expected_json"])?,
                stdin: tutor::loose_text(&t["stdin"]),
                expected_output: tutor::loose_text(&t["expected_output"]),
                hidden: t["hidden"].as_bool().unwrap_or(false),
            };
            let usable = match kind {
                Kind::Function => !tc.expected.is_null() || t["expected_json"].as_str() == Some("null"),
                Kind::Output => !tc.expected_output.trim().is_empty(),
                Kind::Query => tc.expected.is_array(),
            };
            usable.then_some(tc)
        })
        .take(12)
        .collect();
    if tests.len() < 2 {
        bail!("exercise needs at least two usable tests");
    }
    let setup_sql = text("setup_sql");
    if kind == Kind::Query && setup_sql.trim().is_empty() {
        bail!("SQL exercise is missing its setup tables");
    }
    let mut tests = tests;
    // Always show at least one example and keep at least one hidden check.
    if tests.iter().all(|t| t.hidden) {
        tests[0].hidden = false;
    }
    if tests.len() >= 3 && tests.iter().all(|t| !t.hidden) {
        let last = tests.len() - 1;
        tests[last].hidden = true;
    }
    Ok(Exercise {
        title: text("title"),
        prompt: text("prompt"),
        language,
        kind,
        function_name,
        starter_code: v["starter_code"].as_str().unwrap_or_default().to_string(),
        setup_sql,
        ordered: v["ordered"].as_bool().unwrap_or(false),
        tests,
    })
}

pub async fn generate(
    cfg: &Config,
    db: &Mutex<Connection>,
    provider: &Provider,
    track: &Track,
    lesson_id: i64,
) -> Result<(i64, crate::llm::Usage)> {
    let lesson = tutor::get_lesson(&db.lock().unwrap(), lesson_id)?;
    let n = lesson.objectives.len();
    let topic_i = (lesson.topic_idx.max(0) as usize).min(n.saturating_sub(1));
    let topic = lesson.objectives.get(topic_i).cloned().unwrap_or_else(|| lesson.title.clone());
    let previous: Vec<String> = {
        let conn = db.lock().unwrap();
        let mut s = conn.prepare("SELECT title FROM exercises WHERE lesson_id = ?1")?;
        s.query_map([lesson_id], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?
    };
    let sources = rag::search(cfg, db, track.id, &format!("{} {topic}", lesson.title), 3).await.unwrap_or_default();

    let system = "You write short, well-specified coding exercises for a student learning to become a backend / AI engineer. \
Choose the language from the lesson: SQL topics use language \"sql\" with kind \"query\"; JavaScript/Node topics use \"javascript\"; \
Java topics use \"java\" with kind \"output\"; everything else uses \"python\".\n\
Kinds:\n\
- \"function\" (python/javascript): the learner implements function_name. Each test gives args_json (a JSON array of arguments) \
and expected_json (the JSON return value). Prefer this kind.\n\
- \"output\": the learner writes a whole program reading stdin and printing. Each test gives stdin and expected_output.\n\
- \"query\": setup_sql creates and fills small SQLite tables (CREATE TABLE + INSERT only). The learner writes one SELECT. \
Each test's expected_json is a JSON array of result rows, each row an array of column values. Set ordered true only if ORDER BY matters.\n\
Rules: 4-6 tests covering normal and edge cases; mark 1-2 as hidden. Work out every expected value carefully by hand; a wrong \
test is worse than no test. starter_code is a stub with the signature and a TODO, never the solution. The prompt is Markdown, \
under 150 words, states inputs, outputs and one worked example. Use empty strings for fields a kind doesn't use. No emoji.";
    let user = format!(
        "Track: {}\nLesson: {} (module {})\nTopic to practise: {topic}\nAll lesson topics: {}\n\
Earlier exercises in this lesson (don't repeat): {}\n\nRelevant excerpts from the learner's files:\n{}",
        track.name,
        lesson.title,
        lesson.module,
        lesson.objectives.join("; "),
        if previous.is_empty() { "none".into() } else { previous.join("; ") },
        tutor::format_sources(&sources),
    );

    let mut last_err = None;
    for _ in 0..2 {
        let (v, usage) = provider.complete_json(system, &user, exercise_schema()).await?;
        match parse_exercise(&v) {
            Ok(ex) => {
                let conn = db.lock().unwrap();
                conn.execute(
                    "INSERT INTO exercises (lesson_id, topic_idx, title, body, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![lesson_id, topic_i as i64, ex.title, serde_json::to_string(&ex)?, db::now()],
                )?;
                // Read the id before any other insert (usage) changes last_insert_rowid.
                let id = conn.last_insert_rowid();
                db::record_usage(&conn, &usage)?;
                return Ok((id, usage));
            }
            Err(e) => last_err = Some(e),
        }
    }
    bail!("Couldn't build a usable exercise ({}). Try again.", last_err.map(|e| e.to_string()).unwrap_or_default())
}

// ---------- Storage ----------

pub fn load(conn: &Connection, id: i64) -> Result<(i64, Exercise)> {
    let (lesson_id, body): (i64, String) =
        conn.query_row("SELECT lesson_id, body FROM exercises WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok((lesson_id, serde_json::from_str(&body)?))
}

pub fn view(conn: &Connection, id: i64) -> Result<ExerciseView> {
    let (lesson_id, topic_idx, draft, body): (i64, i64, Option<String>, String) = conn.query_row(
        "SELECT lesson_id, topic_idx, draft, body FROM exercises WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let ex: Exercise = serde_json::from_str(&body)?;
    let (attempts, best): (i64, Option<i64>) = conn.query_row(
        "SELECT COUNT(*), MAX(passed) FROM attempts WHERE exercise_id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(ExerciseView {
        id,
        lesson_id,
        topic_idx,
        title: ex.title,
        prompt: ex.prompt,
        language: ex.language,
        kind: ex.kind,
        function_name: ex.function_name,
        starter_code: ex.starter_code,
        setup_sql: ex.setup_sql,
        draft,
        examples: ex.tests.iter().filter(|t| !t.hidden).cloned().collect(),
        hidden_count: ex.tests.iter().filter(|t| t.hidden).count(),
        best: best.map(|b| (b, ex.tests.len() as i64)),
        attempts,
    })
}

pub fn list(conn: &Connection, lesson_id: i64) -> Result<Vec<ExerciseView>> {
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM exercises WHERE lesson_id = ?1 ORDER BY id")?
        .query_map([lesson_id], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    ids.into_iter().map(|id| view(conn, id)).collect()
}

pub fn save_draft(conn: &Connection, id: i64, code: &str) -> Result<()> {
    conn.execute("UPDATE exercises SET draft = ?1 WHERE id = ?2", params![code, id])?;
    Ok(())
}

// ---------- Feedback ----------

pub fn feedback_prompt(ex: &Exercise, code: &str, grade: &Grade, attempt_no: i64) -> (String, String) {
    let system = "You are a code reviewer and tutor. The learner just submitted code for an exercise and it was run \
against tests on their machine. Reply in under 180 words of Markdown:\n\
1. One line verdict.\n2. If tests failed: the specific bug or misconception, pointing at their code, and a hint toward the fix. \
Do not write the corrected solution unless this is attempt 3 or later; then you may show the key lines.\n\
3. If everything passed: one thing done well and one improvement in style, naming, edge cases or complexity.\n\
Never reveal the exact values of hidden tests. If a test's expected value itself looks wrong, say so plainly. No emoji.";
    let tests: Vec<String> = grade
        .results
        .iter()
        .zip(&ex.tests)
        .enumerate()
        .map(|(i, (r, t))| {
            let status = if r.passed { "PASS" } else { "FAIL" };
            let detail = match ex.kind {
                Kind::Function => format!("args={} expected={} got={}", Value::Array(t.args.clone()), t.expected, r.got.clone().unwrap_or_else(|| "?".into())),
                Kind::Output => format!("stdin={:?} expected={:?} got={:?}", t.stdin, t.expected_output, r.got.clone().unwrap_or_default()),
                Kind::Query => format!("expected rows={} got={}", t.expected, r.got.clone().unwrap_or_default()),
            };
            format!("{} test {} {}{}: {}{}", status, i + 1, if t.hidden { "(hidden) " } else { "" }, "", detail, r.error.as_ref().map(|e| format!(" error={e}")).unwrap_or_default())
        })
        .collect();
    let user = format!(
        "Exercise: {}\n{}\n\nLanguage: {:?}\nAttempt number: {attempt_no}\n\nLearner's code:\n```\n{code}\n```\n\n\
Result: {}/{} tests passed.{}\n{}",
        ex.title,
        ex.prompt,
        ex.language,
        grade.passed,
        grade.total,
        grade.error.as_ref().map(|e| format!("\nProgram error: {e}")).unwrap_or_default(),
        tests.join("\n"),
    );
    // Put the hint-only rule next to the code: models follow it far more reliably there.
    let rule = if attempt_no < 3 {
        format!(
            "\n\nIMPORTANT: this is attempt {attempt_no}. Do NOT write the corrected code, not even one fixed line or \
expression. Explain what is wrong and give a hint the learner can act on in words."
        )
    } else {
        "\n\nThe learner has tried 3+ times: you may show the key corrected lines and explain them.".to_string()
    };
    (system.into(), user + &rule)
}

/// True when the learner submitted the stub unchanged; no point spending a model call on it.
pub fn is_untouched(ex: &Exercise, code: &str) -> bool {
    let squash = |s: &str| s.split_whitespace().collect::<String>();
    code.trim().is_empty() || squash(code) == squash(&ex.starter_code)
}

pub const UNTOUCHED_REVIEW: &str = "**Not started yet.** This is still the starter code. Replace the `TODO` with your \
solution, press **Ctrl+Enter** to try it, then submit again.";

pub fn record_attempt(conn: &Connection, exercise_id: i64, code: &str, grade: &Grade, feedback: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO attempts (exercise_id, code, passed, total, grade, feedback, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![exercise_id, code, grade.passed as i64, grade.total as i64, serde_json::to_string(grade)?, feedback, db::now()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn attempt_count(conn: &Connection, exercise_id: i64) -> Result<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM attempts WHERE exercise_id = ?1", [exercise_id], |r| r.get(0))?)
}

// ---------- Study notes ----------

pub fn notes_dir() -> PathBuf {
    if let Ok(d) = std::env::var("NOTES_DIR") {
        if !d.trim().is_empty() {
            return PathBuf::from(d);
        }
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Documents").join("CPrep Notes")
}

pub fn safe_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() || " -_&.()".contains(c) { c } else { ' ' })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect::<String>().trim_end_matches('.').to_string()
}

pub async fn write_notes(db: &Mutex<Connection>, provider: &Provider, track: &Track, lesson_id: i64) -> Result<(PathBuf, crate::llm::Usage)> {
    let (lesson, transcript, sources, exercises) = {
        let conn = db.lock().unwrap();
        let lesson = tutor::get_lesson(&conn, lesson_id)?;
        let msgs = tutor::messages(&conn, lesson_id)?;
        let mut sources: Vec<String> = msgs
            .iter()
            .flat_map(|m| m.sources.iter().map(|s| format!("{}{}", s.title, s.page.map(|p| format!(", p. {p}")).unwrap_or_default())))
            .collect();
        sources.sort();
        sources.dedup();
        let mut transcript = String::new();
        for m in &msgs {
            if m.content.trim().is_empty() {
                continue;
            }
            transcript.push_str(if m.role == "user" { "\nLEARNER: " } else { "\nTUTOR: " });
            transcript.push_str(&m.content);
            transcript.push('\n');
        }
        // Keep the most recent part if the conversation is long.
        if transcript.len() > 24_000 {
            let mut cut = transcript.len() - 24_000;
            while !transcript.is_char_boundary(cut) {
                cut += 1;
            }
            transcript = transcript[cut..].to_string();
        }
        let exercises: Vec<String> = conn
            .prepare("SELECT e.title, MAX(a.passed), MAX(a.total) FROM exercises e LEFT JOIN attempts a ON a.exercise_id = e.id WHERE e.lesson_id = ?1 GROUP BY e.id")?
            .query_map([lesson_id], |r| {
                let t: String = r.get(0)?;
                let p: Option<i64> = r.get(1)?;
                let n: Option<i64> = r.get(2)?;
                Ok(match (p, n) {
                    (Some(p), Some(n)) => format!("{t} ({p}/{n} tests passed)"),
                    _ => format!("{t} (not attempted)"),
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        (lesson, transcript, sources, exercises)
    };
    if transcript.trim().is_empty() {
        bail!("Start the lesson first. Notes are written from what you covered.");
    }
    let system = "You turn a tutoring session into concise, well-organised study notes the learner will revise from later. \
Write Markdown only: a short intro line, then one `##` section per topic with the key ideas as tight bullets, a small code \
example where useful (fenced, with language), and common mistakes. End with `## Check yourself` (4-6 short questions, answers \
in a collapsed <details> block) and `## Sources` listing the files given. Only include what was actually covered or is needed \
to understand it. Don't repeat the lesson title as a heading or subtitle; start with the intro line. No emoji, no filler.";
    let user = format!(
        "Lesson: {}\nTrack: {} / {} / {}\nTopics: {}\nExercises: {}\nSources used: {}\n\nSession transcript:\n{}",
        lesson.title,
        track.name,
        lesson.phase,
        lesson.module,
        lesson.objectives.join("; "),
        if exercises.is_empty() { "none".into() } else { exercises.join("; ") },
        if sources.is_empty() { "none".into() } else { sources.join("; ") },
        transcript
    );
    let (body, usage) = provider.stream_chat(system, &[ChatMsg { role: "user".into(), content: user }], |_| {}).await?;
    let body = body.trim();
    if body.is_empty() {
        bail!("The model returned empty notes. Try again.");
    }
    let dir = notes_dir().join(safe_name(track.name));
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{:02} {}.md", lesson.ord + 1, safe_name(&lesson.title)));
    let header = format!(
        "# {}\n\n_{} · {} · {} · written {}_\n\n",
        lesson.title,
        track.name,
        lesson.phase,
        lesson.module,
        chrono::Local::now().format("%Y-%m-%d")
    );
    let body = body.strip_prefix(&format!("# {}", lesson.title)).unwrap_or(body).trim_start();
    std::fs::write(&file, format!("{header}{body}\n"))?;
    {
        let conn = db.lock().unwrap();
        db::record_usage(&conn, &usage)?;
        conn.execute(
            "INSERT INTO notes (lesson_id, path, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(lesson_id) DO UPDATE SET path = ?2, created_at = ?3",
            params![lesson_id, file.to_string_lossy(), db::now()],
        )?;
    }
    Ok((file, usage))
}

pub fn notes_path(conn: &Connection, lesson_id: i64) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT path FROM notes WHERE lesson_id = ?1", [lesson_id], |r| r.get::<_, String>(0))
        .optional()?
        .filter(|p| std::path::Path::new(p).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_exercise_and_fixes_combos() {
        let v = json!({
            "title": "Sum evens", "prompt": "Return the sum of even numbers.", "language": "python", "kind": "function",
            "function_name": "sum_evens", "starter_code": "def sum_evens(nums):\n    pass\n", "setup_sql": "", "ordered": false,
            "tests": [
                {"args_json": "[[1,2,3,4]]", "expected_json": "6", "stdin": "", "expected_output": "", "hidden": false},
                {"args_json": "[[]]", "expected_json": "0", "stdin": "", "expected_output": "", "hidden": false},
                {"args_json": "[[2,2]]", "expected_json": "4", "stdin": "", "expected_output": "", "hidden": false},
                {"args_json": "not json", "expected_json": "", "stdin": "", "expected_output": "", "hidden": false}
            ]
        });
        let ex = parse_exercise(&v).unwrap();
        assert_eq!(ex.tests.len(), 3);
        assert!(ex.tests[2].hidden, "at least one check stays hidden");
        assert_eq!(ex.tests[0].args, vec![json!([1, 2, 3, 4])]);

        let sql = json!({
            "title": "Adults", "prompt": "p", "language": "sql", "kind": "function", "function_name": "", "starter_code": "",
            "setup_sql": "CREATE TABLE t(a INT); INSERT INTO t VALUES (1);", "ordered": false,
            "tests": [
                {"args_json": "", "expected_json": "[[1]]", "stdin": "", "expected_output": "", "hidden": false},
                {"args_json": "", "expected_json": "[[1]]", "stdin": "", "expected_output": "", "hidden": true}
            ]
        });
        assert_eq!(parse_exercise(&sql).unwrap().kind, Kind::Query, "SQL is always graded as a query");

        let bad = json!({"title": "x", "prompt": "p", "language": "python", "kind": "function", "function_name": "1bad", "tests": []});
        assert!(parse_exercise(&bad).is_err());
    }

    #[test]
    fn note_file_names_are_safe() {
        assert_eq!(safe_name("Routes/views: <intro>?"), "Routes views intro");
        assert_eq!(safe_name("Data Structures & Algorithms"), "Data Structures & Algorithms");
    }
}
