//! End-to-end run in test mode (local Ollama, no Claude credit).
//! Run with: cargo test --test e2e -- --ignored --nocapture
//! Uses CPREP_E2E_FOLDER (defaults to ~/Desktop/programming/BACKEND JOURNEY).

use cprep_lib::{config::Config, db, ingest, llm, rag, tutor};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

#[tokio::test]
#[ignore]
async fn full_flow_on_real_folder() {
    let folder = std::env::var("CPREP_E2E_FOLDER").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("USERPROFILE").unwrap()).join("Desktop/programming/BACKEND JOURNEY")
    });
    assert!(folder.is_dir(), "missing test folder {folder:?}");
    let cfg = Config::load();
    let tmp = std::env::temp_dir().join(format!("cprep-e2e-{}.db", uuid::Uuid::new_v4()));
    let db = Mutex::new(db::open(&tmp).unwrap());

    // 1. Ingest
    let t0 = Instant::now();
    db.lock()
        .unwrap()
        .execute("INSERT INTO folders (track_id, path) VALUES ('backend', ?1)", [folder.to_string_lossy()])
        .unwrap();
    let report = ingest::ingest_folder(&cfg, &db, 1, &folder, |_| {}).await.unwrap();
    println!("INGEST {:?} in {:.0?}", report, t0.elapsed());
    assert!(report.updated > 10 && report.chunks_embedded > 50);
    let leaked: i64 = db
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM documents WHERE title LIKE '%.env%' OR title LIKE '%.db'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(leaked, 0, ".env / .db files must never be ingested");

    // Re-ingest is incremental.
    let again = ingest::ingest_folder(&cfg, &db, 1, &folder, |_| {}).await.unwrap();
    assert_eq!(again.updated, 0);

    // 2. Retrieval
    let hits = rag::search(&cfg, &db, "backend", "How do I define a route in Flask?", 5).await.unwrap();
    for h in &hits {
        println!("HIT {:.3} {} p{:?}", h.score, h.title, h.page);
    }
    assert!(hits.iter().any(|h| h.title.to_lowercase().contains("flask")), "Flask question should hit Flask material");

    // 3. Placement
    let track = db::track("backend").unwrap();
    let provider = llm::route(&cfg, false, llm::Task::Quiz).unwrap();
    println!("PROVIDER {} (resolved: {})", provider.label(), llm::resolve_ollama_model(&cfg.ollama_host, &cfg.ollama_chat_model).await.unwrap_or_default());
    let t0 = Instant::now();
    let (qs, _) = tutor::generate_quiz(&provider, track, &["Python".into(), "Flask".into()], "").await.unwrap();
    println!("QUIZ {} questions in {:.0?}; first: {}", qs.len(), t0.elapsed(), qs[0].prompt);
    let answers: Vec<Option<usize>> = qs.iter().map(|q| Some(q.answer)).collect();
    let result = tutor::grade(&qs, &answers);
    assert_eq!(result.correct, qs.len());
    db.lock()
        .unwrap()
        .execute(
            "INSERT INTO placements (track_id, known, result, updated_at) VALUES ('backend', '[]', ?1, 'now')",
            [serde_json::to_string(&result).unwrap()],
        )
        .unwrap();

    // 4. Curriculum
    let t0 = Instant::now();
    let library: Vec<String> = {
        let conn = db.lock().unwrap();
        let mut s = conn.prepare("SELECT title FROM documents WHERE kind = 'pdf'").unwrap();
        s.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
    };
    let (c, _) = tutor::design_curriculum(&provider, track, Some(&result), &["Python".into()], &library).await.unwrap();
    println!("CURRICULUM '{}' with {} phases in {:.0?}", c.title, c.phases.len(), t0.elapsed());
    let cid = tutor::save_curriculum(&db.lock().unwrap(), "backend", &c).unwrap();
    let lesson_id: i64 = db
        .lock()
        .unwrap()
        .query_row("SELECT id FROM lessons WHERE curriculum_id = ?1 AND status = 'todo' ORDER BY ord LIMIT 1", [cid], |r| r.get(0))
        .unwrap();

    // 5. Lesson: start + one learner reply, streamed.
    let t0 = Instant::now();
    let mut deltas = 0;
    tutor::chat_turn(&cfg, &db, &provider, track, lesson_id, "", |e| {
        if let tutor::ChatEvent::Delta { .. } = e {
            deltas += 1
        }
    })
    .await
    .unwrap();
    assert!(deltas > 5, "reply should stream in pieces");
    tutor::chat_turn(&cfg, &db, &provider, track, lesson_id, "I think it's a decorator on a function?", |_| {})
        .await
        .unwrap();
    let msgs = tutor::messages(&db.lock().unwrap(), lesson_id).unwrap();
    println!("LESSON 2 turns in {:.0?}", t0.elapsed());
    println!("--- tutor opening ---\n{}\n---", msgs[1].content);
    assert_eq!(msgs.len(), 4);
    let lesson = tutor::get_lesson(&db.lock().unwrap(), lesson_id).unwrap();
    println!("TOPICS {:?}", lesson.objectives);
    println!("TOPIC MARKERS: reply1={:?} reply2={:?} lesson.topic_idx={}", msgs[1].topic_idx, msgs[3].topic_idx, lesson.topic_idx);
    assert!(msgs[1].topic_idx.is_some(), "tutor should tag replies with a topic");
    assert!(!msgs[1].content.contains("[[topic"), "markers must be stripped from what the learner sees");
    assert!(!msgs[1].sources.is_empty(), "lesson should retrieve from the learner's files");

    drop(db);
    let _ = std::fs::remove_file(&tmp);
}

/// Quick check of the Gemini free tier (backup test provider): one JSON call and one streamed reply.
/// Run with: cargo test --test e2e gemini -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn gemini_smoke() {
    let mut cfg = Config::load();
    cfg.test_provider = "gemini".into();
    let provider = llm::route(&cfg, false, llm::Task::Quiz).unwrap();
    println!("PROVIDER {}", provider.label());
    let track = db::track("networking").unwrap();
    let t0 = Instant::now();
    let (qs, _) = tutor::generate_quiz(&provider, track, &["TCP/UDP".into()], "").await.unwrap();
    println!("QUIZ {} questions in {:.0?}", qs.len(), t0.elapsed());
    let mut pieces = 0;
    let (reply, _) = provider
        .stream_chat("You are a concise tutor.", &[llm::ChatMsg { role: "user".into(), content: "What is a subnet mask, in two sentences?".into(), image_png: None }], |_| pieces += 1)
        .await
        .unwrap();
    println!("STREAM {pieces} pieces: {reply}");
    assert!(!reply.trim().is_empty());
}

/// Code lab against the configured test provider: generate an exercise, grade the starter code,
/// stream a review, and write study notes. No folder ingest needed.
/// Run with: cargo test --test e2e lab_flow -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn lab_flow() {
    let cfg = Config::load();
    let tmp = std::env::temp_dir().join(format!("cprep-lab-{}.db", uuid::Uuid::new_v4()));
    let db = Mutex::new(db::open(&tmp).unwrap());
    let track = db::track("backend").unwrap();
    let c = tutor::validate_curriculum(serde_json::json!({
        "title": "Backend", "summary": "s",
        "phases": [{"title": "Core Python", "goal": "g", "modules": [{"title": "Data handling", "tested_out": false, "lessons": [
            {"title": "Lists and dictionaries", "objectives": ["List comprehensions", "Dictionary lookups", "Counting with dicts"]}
        ]}]}]
    }))
    .unwrap();
    let cid = tutor::save_curriculum(&db.lock().unwrap(), "backend", &c).unwrap();
    let lesson_id: i64 = db.lock().unwrap().query_row("SELECT id FROM lessons WHERE curriculum_id = ?1", [cid], |r| r.get(0)).unwrap();
    let provider = llm::route(&cfg, false, llm::Task::Quiz).unwrap();
    println!("PROVIDER {}", provider.label());
    // Pre-existing usage rows make the exercise id differ from the usage row id (regression guard).
    db.lock().unwrap().execute_batch("INSERT INTO usage (provider, model, input_tokens, output_tokens, cache_read, cache_write, cost_usd, created_at) VALUES ('x','x',0,0,0,0,0,'now'), ('x','x',0,0,0,0,0,'now');").unwrap();

    let t0 = Instant::now();
    let (id, _) = cprep_lib::lab::generate(&cfg, &db, &provider, track, lesson_id).await.unwrap();
    let (_, ex) = cprep_lib::lab::load(&db.lock().unwrap(), id).unwrap();
    assert_eq!(cprep_lib::lab::view(&db.lock().unwrap(), id).unwrap().title, ex.title);
    println!("EXERCISE in {:.0?}: {} [{:?}/{:?}] fn={}\n{}\nSTARTER:\n{}", t0.elapsed(), ex.title, ex.language, ex.kind, ex.function_name, ex.prompt, ex.starter_code);
    for t in &ex.tests {
        println!("  TEST hidden={} args={:?} expected={} stdin={:?} out={:?}", t.hidden, t.args, t.expected, t.stdin, t.expected_output);
    }

    let grade = cprep_lib::runner::grade(&ex.language, &ex.kind, &ex.starter_code, &ex.function_name, &ex.setup_sql, ex.ordered, &ex.tests)
        .await
        .unwrap();
    println!("STARTER GRADE {}/{} error={:?}", grade.passed, grade.total, grade.error);
    assert!(grade.passed < grade.total, "the starter stub must not already pass");

    assert!(cprep_lib::lab::is_untouched(&ex, &ex.starter_code));
    // Review a realistic wrong attempt: squares every number instead of only the even ones.
    let header = ex.starter_code.lines().next().unwrap_or_default().to_string();
    let wrong = if ex.language == cprep_lib::runner::Language::Python && header.starts_with("def ") {
        let arg = header.split('(').nth(1).and_then(|a| a.split(&[',', ')'][..]).next()).unwrap_or("nums").trim().to_string();
        format!("{header}\n    return [x * x for x in {arg}]\n")
    } else {
        ex.starter_code.clone()
    };
    let grade = cprep_lib::runner::grade(&ex.language, &ex.kind, &wrong, &ex.function_name, &ex.setup_sql, ex.ordered, &ex.tests)
        .await
        .unwrap();
    println!("WRONG ATTEMPT GRADE {}/{}:\n{wrong}", grade.passed, grade.total);
    let (system, user) = cprep_lib::lab::feedback_prompt(&ex, &wrong, &grade, 1);
    let (review, _) = provider.stream_chat(&system, &[llm::ChatMsg { role: "user".into(), content: user, image_png: None }], |_| {}).await.unwrap();
    println!("REVIEW:\n{review}");
    assert!(!review.trim().is_empty());

    {
        let conn = db.lock().unwrap();
        for (role, text) in [
            ("assistant", "A list comprehension builds a list in one line: `[x * 2 for x in nums if x > 0]`. Check: what does `[c for c in 'abc']` give?"),
            ("user", "['a', 'b', 'c']"),
            ("assistant", "Correct. Dictionaries map keys to values; `d.get(k, 0)` avoids a KeyError. Counting: `counts[w] = counts.get(w, 0) + 1`."),
        ] {
            conn.execute(
                "INSERT INTO messages (lesson_id, role, content, llm_content, created_at) VALUES (?1, ?2, ?3, ?3, 'now')",
                rusqlite::params![lesson_id, role, text],
            )
            .unwrap();
        }
    }
    // SAFETY: set before any other thread reads NOTES_DIR in this test process.
    unsafe { std::env::set_var("NOTES_DIR", std::env::temp_dir().join("cprep-notes-test")) };
    let (path, _) = cprep_lib::lab::write_notes(&db, &provider, track, lesson_id).await.unwrap();
    let notes = std::fs::read_to_string(&path).unwrap();
    println!("NOTES at {}:\n{}", path.display(), notes.chars().take(900).collect::<String>());
    assert!(notes.starts_with("# Lists and dictionaries"));

    drop(db);
    let _ = std::fs::remove_file(&tmp);
}

/// Canvas: a drawing task and a critique of a described (text-only) diagram.
/// Run with: cargo test --test e2e canvas_flow -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn canvas_flow() {
    let cfg = Config::load();
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    db::init(&conn).unwrap();
    let c = tutor::validate_curriculum(serde_json::json!({
        "title": "System design", "summary": "s",
        "phases": [{"title": "Scaling", "goal": "g", "modules": [{"title": "Caching", "tested_out": false, "lessons": [
            {"title": "Caching a read-heavy API", "objectives": ["Cache-aside pattern", "Cache invalidation", "TTL choice"]}
        ]}]}]
    }))
    .unwrap();
    let cid = tutor::save_curriculum(&conn, "system-design", &c).unwrap();
    let lesson = tutor::get_lesson(&conn, conn.query_row("SELECT id FROM lessons WHERE curriculum_id = ?1", [cid], |r| r.get(0)).unwrap()).unwrap();
    let track = db::track("system-design").unwrap();
    let provider = llm::route(&cfg, false, llm::Task::Tutor).unwrap();
    println!("PROVIDER {} (sees images: {})", provider.label(), provider.sees_images());

    let (system, user) = cprep_lib::canvas::task_prompt(track, &lesson);
    let t0 = Instant::now();
    let (task, _) = provider.stream_chat(&system, &[llm::ChatMsg::text("user", user)], |_| {}).await.unwrap();
    println!("TASK in {:.0?}:\n{task}", t0.elapsed());
    assert!(!task.trim().is_empty());

    // A deliberately incomplete design: no cache yet.
    let description = "Shapes (top to bottom, left to right):\n- rectangle \"Client\"\n- rectangle \"API server\"\n- rectangle \"Postgres\"\nConnections:\n- \"Client\" → \"API server\" [label \"GET /products/42\"]\n- \"API server\" → \"Postgres\" [label \"SELECT\"]";
    let (system, user) = cprep_lib::canvas::critique_prompt(track, &lesson, Some(&task), description, false);
    let t0 = Instant::now();
    let (review, _) = provider.stream_chat(&system, &[llm::ChatMsg::text("user", user)], |_| {}).await.unwrap();
    println!("CRITIQUE in {:.0?}:\n{review}", t0.elapsed());
    assert!(review.to_lowercase().contains("cache"), "critique should notice the missing cache");
}
