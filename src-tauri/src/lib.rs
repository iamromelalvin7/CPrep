//! CPrep: Tauri commands wiring the UI to the Rust core.

pub mod config;
pub mod db;
pub mod ingest;
pub mod lab;
pub mod llm;
pub mod rag;
pub mod runner;
pub mod tutor;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::ipc::Channel;
use tauri::{Emitter, Manager, State};

use config::Config;
use llm::Task;
use tutor::{ChatEvent, PlacementResult, PublicQuestion, Question};

pub struct AppState {
    pub db: Mutex<Connection>,
    pub cfg: Config,
    pub live: AtomicBool,
}

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn track(id: &str) -> CmdResult<&'static db::Track> {
    db::track(id).ok_or_else(|| format!("Unknown track: {id}"))
}

// ---------- Overview & settings ----------

#[derive(Serialize)]
struct TrackSummary {
    id: &'static str,
    name: &'static str,
    blurb: &'static str,
    accent: &'static str,
    status: &'static str,
    folders: i64,
    documents: i64,
    lessons_total: i64,
    lessons_done: i64,
}

#[tauri::command]
fn list_tracks(state: State<'_, AppState>) -> CmdResult<Vec<TrackSummary>> {
    let conn = state.db.lock().unwrap();
    db::TRACKS
        .iter()
        .map(|t| {
            let folders: i64 = conn
                .query_row("SELECT COUNT(*) FROM folders WHERE track_id = ?1", [t.id], |r| r.get(0))
                .map_err(err)?;
            let documents: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM documents d JOIN folders f ON f.id = d.folder_id WHERE f.track_id = ?1",
                    [t.id],
                    |r| r.get(0),
                )
                .map_err(err)?;
            let (total, done): (i64, i64) = conn
                .query_row(
                    "SELECT COUNT(*), COALESCE(SUM(status IN ('done','skipped')), 0) FROM lessons
                     WHERE curriculum_id = (SELECT MAX(id) FROM curricula WHERE track_id = ?1)",
                    [t.id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(err)?;
            Ok(TrackSummary {
                id: t.id,
                name: t.name,
                blurb: t.blurb,
                accent: t.accent,
                status: t.status,
                folders,
                documents,
                lessons_total: total,
                lessons_done: done,
            })
        })
        .collect()
}

#[derive(Serialize)]
struct Settings {
    live: bool,
    has_anthropic: bool,
    has_gemini: bool,
    has_groq: bool,
    env_path: Option<String>,
    test_provider: String,
    tutor_model: String,
    curriculum_model: String,
    escalate_model: String,
    ollama_chat_model: String,
    ollama_embed_model: String,
    ollama_up: bool,
    notes_dir: String,
    ollama_models: Vec<String>,
    spend_usd: f64,
}

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> CmdResult<Settings> {
    let cfg = &state.cfg;
    let models = llm::ollama_models(&cfg.ollama_host).await;
    let spend: f64 = state
        .db
        .lock()
        .unwrap()
        .query_row("SELECT COALESCE(SUM(cost_usd), 0) FROM usage", [], |r| r.get(0))
        .map_err(err)?;
    Ok(Settings {
        live: state.live.load(Ordering::Relaxed),
        has_anthropic: cfg.anthropic_key.is_some(),
        has_gemini: cfg.gemini_key.is_some(),
        has_groq: cfg.groq_key.is_some(),
        env_path: cfg.env_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        test_provider: cfg.test_provider.clone(),
        tutor_model: cfg.tutor_model.clone(),
        curriculum_model: cfg.curriculum_model.clone(),
        escalate_model: cfg.escalate_model.clone(),
        ollama_chat_model: cfg.ollama_chat_model.clone(),
        ollama_embed_model: cfg.ollama_embed_model.clone(),
        ollama_up: models.is_ok(),
        notes_dir: lab::notes_dir().to_string_lossy().to_string(),
        ollama_models: models.unwrap_or_default(),
        spend_usd: spend,
    })
}

#[tauri::command]
fn set_live(state: State<'_, AppState>, live: bool) -> CmdResult<()> {
    if live && state.cfg.anthropic_key.is_none() {
        return Err("Add ANTHROPIC_API_KEY to your .env file, then restart CPrep.".into());
    }
    state.live.store(live, Ordering::Relaxed);
    state
        .db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO settings (key, value) VALUES ('live', ?1) ON CONFLICT(key) DO UPDATE SET value = ?1",
            [if live { "1" } else { "0" }],
        )
        .map_err(err)?;
    Ok(())
}

// ---------- Track detail ----------

#[derive(Serialize)]
struct Folder {
    id: i64,
    path: String,
    last_ingested: Option<String>,
    documents: i64,
    chunks: i64,
}

#[derive(Serialize)]
struct CurriculumView {
    id: i64,
    title: String,
    summary: String,
    phases: Vec<Value>,
    lessons: Vec<tutor::Lesson>,
}

#[derive(Serialize)]
struct TrackDetail {
    folders: Vec<Folder>,
    known: Vec<String>,
    placement: Option<PlacementResult>,
    curriculum: Option<CurriculumView>,
}

#[tauri::command]
fn get_track(state: State<'_, AppState>, track_id: String) -> CmdResult<TrackDetail> {
    track(&track_id)?;
    let conn = state.db.lock().unwrap();
    let folders = conn
        .prepare(
            "SELECT f.id, f.path, f.last_ingested,
                (SELECT COUNT(*) FROM documents d WHERE d.folder_id = f.id),
                (SELECT COUNT(*) FROM chunks c JOIN documents d ON d.id = c.doc_id WHERE d.folder_id = f.id)
             FROM folders f WHERE f.track_id = ?1 ORDER BY f.id",
        )
        .map_err(err)?
        .query_map([&track_id], |r| {
            Ok(Folder { id: r.get(0)?, path: r.get(1)?, last_ingested: r.get(2)?, documents: r.get(3)?, chunks: r.get(4)? })
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    let known: Vec<String> = conn
        .query_row("SELECT known FROM placements WHERE track_id = ?1", [&track_id], |r| r.get::<_, String>(0))
        .optional()
        .map_err(err)?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let placement = tutor::load_placement_result(&conn, &track_id).map_err(err)?;
    let curriculum = conn
        .query_row(
            "SELECT id, body FROM curricula WHERE track_id = ?1 ORDER BY id DESC LIMIT 1",
            [&track_id],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(err)?
        .map(|(id, body)| -> CmdResult<CurriculumView> {
            let v: Value = serde_json::from_str(&body).map_err(err)?;
            let lessons = conn
                .prepare(&format!("SELECT {} FROM lessons WHERE curriculum_id = ?1 ORDER BY ord", tutor::LESSON_COLS))
                .map_err(err)?
                .query_map([id], tutor::lesson_from_row)
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?;
            Ok(CurriculumView {
                id,
                title: v["title"].as_str().unwrap_or_default().into(),
                summary: v["summary"].as_str().unwrap_or_default().into(),
                phases: v["phases"].as_array().cloned().unwrap_or_default(),
                lessons,
            })
        })
        .transpose()?;
    Ok(TrackDetail { folders, known, placement, curriculum })
}

#[tauri::command]
fn add_folder(state: State<'_, AppState>, track_id: String, path: String) -> CmdResult<i64> {
    track(&track_id)?;
    if !PathBuf::from(&path).is_dir() {
        return Err(format!("Not a folder: {path}"));
    }
    let conn = state.db.lock().unwrap();
    conn.execute("INSERT OR IGNORE INTO folders (track_id, path) VALUES (?1, ?2)", params![track_id, path])
        .map_err(err)?;
    conn.query_row("SELECT id FROM folders WHERE track_id = ?1 AND path = ?2", params![track_id, path], |r| r.get(0))
        .map_err(err)
}

#[tauri::command]
fn remove_folder(state: State<'_, AppState>, folder_id: i64) -> CmdResult<()> {
    state.db.lock().unwrap().execute("DELETE FROM folders WHERE id = ?1", [folder_id]).map_err(err)?;
    Ok(())
}

#[tauri::command]
async fn ingest_folder(app: tauri::AppHandle, state: State<'_, AppState>, folder_id: i64) -> CmdResult<ingest::Report> {
    let path: String = state
        .db
        .lock()
        .unwrap()
        .query_row("SELECT path FROM folders WHERE id = ?1", [folder_id], |r| r.get(0))
        .map_err(err)?;
    ingest::ingest_folder(&state.cfg, &state.db, folder_id, &PathBuf::from(path), |p| {
        let _ = app.emit("ingest-progress", p);
    })
    .await
    .map_err(err)
}

// ---------- Placement ----------

#[tauri::command]
async fn start_placement(
    state: State<'_, AppState>,
    track_id: String,
    known: Vec<String>,
    note: String,
) -> CmdResult<Vec<PublicQuestion>> {
    let t = track(&track_id)?;
    let provider = llm::route(&state.cfg, state.live.load(Ordering::Relaxed), Task::Quiz).map_err(err)?;
    let (qs, usage) = tutor::generate_quiz(&provider, t, &known, &note).await.map_err(err)?;
    let conn = state.db.lock().unwrap();
    db::record_usage(&conn, &usage).map_err(err)?;
    conn.execute(
        "INSERT INTO placements (track_id, known, quiz, result, updated_at) VALUES (?1, ?2, ?3, NULL, ?4)
         ON CONFLICT(track_id) DO UPDATE SET known = ?2, quiz = ?3, result = NULL, updated_at = ?4",
        params![track_id, serde_json::to_string(&known).map_err(err)?, serde_json::to_string(&qs).map_err(err)?, db::now()],
    )
    .map_err(err)?;
    Ok(qs.into_iter().map(|q| PublicQuestion { topic: q.topic, prompt: q.prompt, options: q.options }).collect())
}

#[tauri::command]
fn submit_placement(state: State<'_, AppState>, track_id: String, answers: Vec<Option<usize>>) -> CmdResult<PlacementResult> {
    let conn = state.db.lock().unwrap();
    let quiz: Option<String> = conn
        .query_row("SELECT quiz FROM placements WHERE track_id = ?1", [&track_id], |r| r.get(0))
        .optional()
        .map_err(err)?
        .flatten();
    let qs: Vec<Question> = serde_json::from_str(&quiz.ok_or("Start the placement quiz first.")?).map_err(err)?;
    let result = tutor::grade(&qs, &answers);
    conn.execute(
        "UPDATE placements SET result = ?1, updated_at = ?2 WHERE track_id = ?3",
        params![serde_json::to_string(&result).map_err(err)?, db::now(), track_id],
    )
    .map_err(err)?;
    Ok(result)
}

#[tauri::command]
fn skip_placement(state: State<'_, AppState>, track_id: String, known: Vec<String>) -> CmdResult<()> {
    state
        .db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO placements (track_id, known, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(track_id) DO UPDATE SET known = ?2, updated_at = ?3",
            params![track_id, serde_json::to_string(&known).map_err(err)?, db::now()],
        )
        .map_err(err)?;
    Ok(())
}

// ---------- Curriculum ----------

#[tauri::command]
async fn generate_curriculum(state: State<'_, AppState>, track_id: String) -> CmdResult<i64> {
    let t = track(&track_id)?;
    let (placement, known, library) = {
        let conn = state.db.lock().unwrap();
        let placement = tutor::load_placement_result(&conn, &track_id).map_err(err)?;
        let known: Vec<String> = conn
            .query_row("SELECT known FROM placements WHERE track_id = ?1", [&track_id], |r| r.get::<_, String>(0))
            .optional()
            .map_err(err)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let library: Vec<String> = conn
            .prepare(
                "SELECT d.title FROM documents d JOIN folders f ON f.id = d.folder_id
                 WHERE f.track_id = ?1 ORDER BY (d.kind = 'pdf') DESC, d.title",
            )
            .map_err(err)?
            .query_map([&track_id], |r| r.get(0))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        (placement, known, library)
    };
    let provider = llm::route(&state.cfg, state.live.load(Ordering::Relaxed), Task::Curriculum).map_err(err)?;
    let (c, usage) = tutor::design_curriculum(&provider, t, placement.as_ref(), &known, &library).await.map_err(err)?;
    let conn = state.db.lock().unwrap();
    db::record_usage(&conn, &usage).map_err(err)?;
    tutor::save_curriculum(&conn, &track_id, &c).map_err(err)
}

// ---------- Lessons ----------

#[derive(Serialize)]
struct LessonView {
    lesson: tutor::Lesson,
    messages: Vec<tutor::Message>,
    prev_id: Option<i64>,
    next_id: Option<i64>,
}

#[tauri::command]
fn get_lesson(state: State<'_, AppState>, lesson_id: i64) -> CmdResult<LessonView> {
    let conn = state.db.lock().unwrap();
    let lesson = tutor::get_lesson(&conn, lesson_id).map_err(err)?;
    let messages = tutor::messages(&conn, lesson_id).map_err(err)?;
    let neighbour = |cmp: &str, order: &str| -> CmdResult<Option<i64>> {
        conn.query_row(
            &format!("SELECT id FROM lessons WHERE curriculum_id = ?1 AND ord {cmp} ?2 ORDER BY ord {order} LIMIT 1"),
            params![lesson.curriculum_id, lesson.ord],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)
    };
    let prev_id = neighbour("<", "DESC")?;
    let next_id = neighbour(">", "ASC")?;
    Ok(LessonView { lesson, messages, prev_id, next_id })
}

#[tauri::command]
fn set_lesson_status(state: State<'_, AppState>, lesson_id: i64, status: String) -> CmdResult<()> {
    if !["todo", "in_progress", "done", "skipped"].contains(&status.as_str()) {
        return Err(format!("bad status {status}"));
    }
    state
        .db
        .lock()
        .unwrap()
        .execute("UPDATE lessons SET status = ?1 WHERE id = ?2", params![status, lesson_id])
        .map_err(err)?;
    Ok(())
}

/// Retake: clear the conversation and start the lesson over.
#[tauri::command]
fn retake_lesson(state: State<'_, AppState>, lesson_id: i64) -> CmdResult<()> {
    let conn = state.db.lock().unwrap();
    conn.execute("DELETE FROM messages WHERE lesson_id = ?1", [lesson_id]).map_err(err)?;
    conn.execute("UPDATE lessons SET status = 'todo', topic_idx = 0 WHERE id = ?1", [lesson_id]).map_err(err)?;
    Ok(())
}

/// Send a message (empty text = start the lesson). Streams ChatEvents over `on_event`.
#[tauri::command]
async fn send_message(
    state: State<'_, AppState>,
    lesson_id: i64,
    text: String,
    escalate: bool,
    on_event: Channel<ChatEvent>,
) -> CmdResult<()> {
    let track_id: String = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT c.track_id FROM lessons l JOIN curricula c ON c.id = l.curriculum_id WHERE l.id = ?1",
            [lesson_id],
            |r| r.get(0),
        )
        .map_err(err)?;
    let t = track(&track_id)?;
    let task = if escalate { Task::Escalate } else { Task::Tutor };
    let provider = llm::route(&state.cfg, state.live.load(Ordering::Relaxed), task).map_err(err)?;
    let result = tutor::chat_turn(&state.cfg, &state.db, &provider, t, lesson_id, &text, |e| {
        let _ = on_event.send(e);
    })
    .await;
    if let Err(e) = result {
        let _ = on_event.send(ChatEvent::Error { message: e.to_string() });
        return Err(err(e));
    }
    Ok(())
}

// ---------- Code lab ----------

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum LabEvent {
    Graded { grade: runner::Grade },
    Delta { text: String },
    Done { attempt_id: i64, provider: String, cost_usd: f64 },
    Error { message: String },
}

fn lesson_track(state: &AppState, lesson_id: i64) -> CmdResult<&'static db::Track> {
    let track_id: String = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT c.track_id FROM lessons l JOIN curricula c ON c.id = l.curriculum_id WHERE l.id = ?1",
            [lesson_id],
            |r| r.get(0),
        )
        .map_err(err)?;
    track(&track_id)
}

#[tauri::command]
fn list_exercises(state: State<'_, AppState>, lesson_id: i64) -> CmdResult<Vec<lab::ExerciseView>> {
    lab::list(&state.db.lock().unwrap(), lesson_id).map_err(err)
}

#[tauri::command]
async fn new_exercise(state: State<'_, AppState>, lesson_id: i64) -> CmdResult<lab::ExerciseView> {
    let t = lesson_track(&state, lesson_id)?;
    let provider = llm::route(&state.cfg, state.live.load(Ordering::Relaxed), Task::Quiz).map_err(err)?;
    let (id, _) = lab::generate(&state.cfg, &state.db, &provider, t, lesson_id).await.map_err(err)?;
    lab::view(&state.db.lock().unwrap(), id).map_err(err)
}

#[tauri::command]
async fn run_code(state: State<'_, AppState>, exercise_id: i64, code: String, stdin: String) -> CmdResult<runner::RunOutput> {
    let (_, ex) = {
        let conn = state.db.lock().unwrap();
        lab::save_draft(&conn, exercise_id, &code).map_err(err)?;
        lab::load(&conn, exercise_id).map_err(err)?
    };
    runner::run(&ex.language, &code, &stdin, &ex.setup_sql).await.map_err(err)
}

#[tauri::command]
fn save_draft(state: State<'_, AppState>, exercise_id: i64, code: String) -> CmdResult<()> {
    lab::save_draft(&state.db.lock().unwrap(), exercise_id, &code).map_err(err)
}

/// Grade the code locally, then stream the tutor's review.
#[tauri::command]
async fn submit_code(state: State<'_, AppState>, exercise_id: i64, code: String, on_event: Channel<LabEvent>) -> CmdResult<()> {
    let (lesson_id, ex, attempt_no) = {
        let conn = state.db.lock().unwrap();
        lab::save_draft(&conn, exercise_id, &code).map_err(err)?;
        let (lesson_id, ex) = lab::load(&conn, exercise_id).map_err(err)?;
        let n = lab::attempt_count(&conn, exercise_id).map_err(err)? + 1;
        (lesson_id, ex, n)
    };
    let grade = runner::grade(&ex.language, &ex.kind, &code, &ex.function_name, &ex.setup_sql, ex.ordered, &ex.tests)
        .await
        .map_err(err)?;
    let _ = on_event.send(LabEvent::Graded { grade: grade.clone() });

    let _ = lesson_track(&state, lesson_id)?;
    if lab::is_untouched(&ex, &code) {
        let _ = on_event.send(LabEvent::Delta { text: lab::UNTOUCHED_REVIEW.into() });
        let conn = state.db.lock().unwrap();
        let id = lab::record_attempt(&conn, exercise_id, &code, &grade, lab::UNTOUCHED_REVIEW).map_err(err)?;
        let _ = on_event.send(LabEvent::Done { attempt_id: id, provider: "CPrep".into(), cost_usd: 0.0 });
        return Ok(());
    }
    let provider = llm::route(&state.cfg, state.live.load(Ordering::Relaxed), Task::Tutor).map_err(err)?;
    let (system, user) = lab::feedback_prompt(&ex, &code, &grade, attempt_no);
    let result = provider
        .stream_chat(&system, &[llm::ChatMsg { role: "user".into(), content: user }], |t| {
            let _ = on_event.send(LabEvent::Delta { text: t.to_string() });
        })
        .await;
    // The grade counts even if the review fails (e.g. offline).
    let (feedback, usage) = match result {
        Ok(x) => x,
        Err(e) => {
            let conn = state.db.lock().unwrap();
            let id = lab::record_attempt(&conn, exercise_id, &code, &grade, "").map_err(err)?;
            let _ = on_event.send(LabEvent::Error { message: format!("Graded, but the review failed: {e}") });
            let _ = on_event.send(LabEvent::Done { attempt_id: id, provider: provider.label(), cost_usd: 0.0 });
            return Ok(());
        }
    };
    let conn = state.db.lock().unwrap();
    db::record_usage(&conn, &usage).map_err(err)?;
    let id = lab::record_attempt(&conn, exercise_id, &code, &grade, &feedback).map_err(err)?;
    let _ = on_event.send(LabEvent::Done { attempt_id: id, provider: provider.label_for(&usage), cost_usd: usage.cost_usd() });
    Ok(())
}

#[tauri::command]
async fn write_notes(state: State<'_, AppState>, lesson_id: i64) -> CmdResult<String> {
    let t = lesson_track(&state, lesson_id)?;
    let provider = llm::route(&state.cfg, state.live.load(Ordering::Relaxed), Task::Tutor).map_err(err)?;
    let (path, _) = lab::write_notes(&state.db, &provider, t, lesson_id).await.map_err(err)?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn notes_path(state: State<'_, AppState>, lesson_id: i64) -> CmdResult<Option<String>> {
    lab::notes_path(&state.db.lock().unwrap(), lesson_id).map_err(err)
}

/// Open a notes file (or the notes folder) in the default app. Only paths inside the notes folder.
#[tauri::command]
fn open_notes(path: Option<String>) -> CmdResult<()> {
    let root = lab::notes_dir();
    std::fs::create_dir_all(&root).map_err(err)?;
    let target = path.map(PathBuf::from).unwrap_or_else(|| root.clone());
    let canon_root = root.canonicalize().map_err(err)?;
    let canon = target.canonicalize().map_err(err)?;
    if !canon.starts_with(&canon_root) {
        return Err("Only files in your notes folder can be opened from here.".into());
    }
    tauri_plugin_opener::open_path(canon, None::<&str>).map_err(err)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let conn = db::open(&dir.join("cprep.db"))?;
            let live = conn
                .query_row("SELECT value FROM settings WHERE key = 'live'", [], |r| r.get::<_, String>(0))
                .optional()?
                .as_deref()
                == Some("1");
            let cfg = Config::load();
            let live = live && cfg.anthropic_key.is_some();
            app.manage(AppState { db: Mutex::new(conn), cfg, live: AtomicBool::new(live) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_tracks,
            get_settings,
            set_live,
            get_track,
            add_folder,
            remove_folder,
            ingest_folder,
            start_placement,
            submit_placement,
            skip_placement,
            generate_curriculum,
            get_lesson,
            set_lesson_status,
            retake_lesson,
            send_message,
            list_exercises,
            new_exercise,
            run_code,
            save_draft,
            submit_code,
            write_notes,
            notes_path,
            open_notes,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
