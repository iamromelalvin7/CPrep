# CPrep: project context

**Goal:** Personal local-first AI tutor desktop app for career prep (backend, AI eng, networking, AI automation, DSA, system design, interviews). Full design: `PLAN.md`.

**Current state (2026-11-15):** Phase 0 built: Tauri 2 app with folder ingestion + RAG, LLM router (Claude / Ollama / Gemini / Groq), placement quiz, curriculum design, streaming lesson chat with margin citations, settings. Phases 1–5 (code lab, canvas, scheduler/notifications, voice, slides/dashboard) not started.

**Run it:**
- `npm run tauri dev` (Vite on port **1420**, not 5173).
- Browser-only UI dev: `npm run dev` → http://localhost:1420 uses a mock backend (`src/api.ts`), handy for Playwright screenshots.
- Rust tests: `cd src-tauri && cargo test`. Real end-to-end (needs Ollama): `cargo test --test e2e -- --ignored --nocapture` (defaults to `~/Desktop/programming/BACKEND JOURNEY`; override with `CPREP_E2E_FOLDER`).

**Layout:**
- `src-tauri/src/` — `lib.rs` (Tauri commands), `db.rs` (schema + track list/syllabi), `ingest.rs` (walk/extract/chunk/redact/embed), `rag.rs` (cosine search), `llm.rs` (providers, SSE parsing, cost), `tutor.rs` (quiz, curriculum, chat turn), `config.rs` (.env).
- `src/` — `App.tsx` (shell/routing), `views/TrackHome.tsx`, `views/Lesson.tsx`, `views/Settings.tsx`, `api.ts` (typed invoke + browser mock), `styles.css` (tokens).
- Data: SQLite at `%APPDATA%\com.romel.cprep\cprep.db`.

**Decisions:**
- Stack: Tauri 2 (Rust core) + React/TS/Vite UI; Python Kokoro sidecar later.
- Models: test mode = `TEST_PROVIDER` groq (default) | gemini | local (Ollama). Live = Claude Haiku 5.5 tutor, Sonnet 5.5 curriculum, Opus 5.5 "ask the expert". Claude called over raw HTTPS (no Rust SDK), `anthropic-version: 2026-10-21`; no `temperature` (rejected on that version).
- Vector search is brute-force cosine over BLOBs in SQLite (no sqlite-vec yet; fine for a few thousand chunks per track).
- Secrets only in `.env` (gitignored). Ingestion skips `.env*`, `.flaskenv`, DBs, logs, and redacts hard-coded keys in code.
- Design: green-ink dark theme, Newsreader (serif) for headings + tutor text, IBM Plex Sans UI, JetBrains Mono code, lamp-yellow accent. Fonts bundled via @fontsource (offline).
- Commits carry no Claude attribution (user's global rule).

**Test mode = Groq (decided 2026-11-16).** User dropped local chat models (gemma2:9b deleted, qwen3:4b never finished; laptop hit low memory). Only `nomic-embed-text` stays local, for search. Measured: ingest ≈ 2 min (embedding); Groq `gpt-oss-120b` quiz 5 s, course 7 s, lesson turn seconds; Gemini `gemini-3.8-flash` works as backup but quiz took ~195 s. Groq free tier ≈ 8K tokens/min on gpt-oss-120b, so `llm.rs` waits and retries on 429. Models often return objects where strings were asked; `tutor.rs` parses leniently and `llm.rs` recovers Groq's `failed_generation`. Claude key not added yet (live mode untested).

If the app is running, `cargo test` can't overwrite `target/debug/cprep.exe`: use `CARGO_TARGET_DIR=target-test`.

**Lesson topics (2026-11-16):** a lesson's `objectives` are its ordered topics. The tutor starts every reply with `[[topic:N]]` (1-based); `tutor::parse_topic` strips it and stores 0-based `messages.topic_idx`, and `lessons.topic_idx` only moves forward. The user turn carries `<lesson_state>` so the system prompt stays cacheable. In a lesson the sidebar becomes `CourseNav` (topics: done=jump back, current, locked). The sidebar collapses (localStorage `cprep.sidebar`). No emoji: the prompt forbids them and `icons.tsx` swaps any that slip through for SVGs. Tab in the reply box indents; Esc then Tab leaves.

**Next:** Phase 1 — Code lab (Monaco + local runner + AI grading), notes saved as `.md`.
