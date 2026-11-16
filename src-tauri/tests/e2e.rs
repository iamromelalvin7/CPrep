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
        .stream_chat("You are a concise tutor.", &[llm::ChatMsg { role: "user".into(), content: "What is a subnet mask, in two sentences?".into() }], |_| pieces += 1)
        .await
        .unwrap();
    println!("STREAM {pieces} pieces: {reply}");
    assert!(!reply.trim().is_empty());
}
