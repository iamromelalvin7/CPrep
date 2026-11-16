# CPrep: project context

**Goal:** Personal local-first AI tutor desktop app for career prep (backend, AI eng, networking, AI automation, DSA, system design, interviews). Full design: `PLAN.md`.

**Current state (2026-11-15):** Phase 0 built: Tauri 2 app with folder ingestion + RAG, LLM router (Claude / Ollama / Gemini / Groq), placement quiz, curriculum design, streaming lesson chat with margin citations, settings. Phase 1 (code lab + study notes) and Phase 2 (canvas + diagrams) built 2026-11-16. Phases 3–5 (scheduler/notifications, voice, slides/dashboard) not started.

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

**Code lab (Phase 1, 2026-11-16):** `runner.rs` runs ONLY the learner's code (Python via `py -3`, Node, `java File.java`, SQL in in-memory rusqlite with a progress-handler timeout), 10 s limit. AI exercises are data: function tests (`args`/`expected`), output tests (stdin/stdout), query tests (rows), fed through fixed harnesses (`PY_HARNESS`, `JS_HARNESS`). Never execute model-written code. `lab.rs`: exercise generation (strict JSON; tests arrive as JSON strings), review prompt (hints only before attempt 3; the rule is appended next to the code), unchanged-starter short-circuit (no model call), notes written to `Documents/CPrep Notes/<track>/NN <lesson>.md` (override with `NOTES_DIR`). UI: `views/Lab.tsx`, lazy-loaded; Monaco set up in `src/monaco.ts` (core + 4 languages, offline). Ctrl+Enter runs, Ctrl+Shift+Enter submits. Mark complete also writes notes in the background. Read `last_insert_rowid()` before any other insert (usage rows) — that bug shipped once.

**Canvas (Phase 2, 2026-11-16):** the tutor may add one ```mermaid block per reply (prompt rule in `lesson_system_prompt`); `views/Diagram.tsx` renders it lazily with Mermaid `securityLevel: "strict"` (model output) and offers "Open on canvas" via the `CanvasBridge` context. `views/Canvas.tsx` (lazy) is Excalidraw 0.18, scene saved per lesson in `canvases`; Mermaid → shapes via `@excalidraw/mermaid-to-excalidraw`. Fonts are bundled in `public/excalidraw-assets/fonts` (CJK Xiaolai omitted, 13 MB) and `EXCALIDRAW_ASSET_PATH` points there, so it works offline. Critique: `src/describeScene.ts` turns the scene into text (shapes, labels, arrows with bindings or nearest-shape fallback) so text-only Groq can judge it; a PNG goes along only when `Provider::sees_images()`. Canvas critique routes `Task::Vision`: Claude in live mode; in Groq test mode the vision model `GROQ_VISION_MODEL` (default `qwen/qwen3.8-27b`, verified 2026-11-16 to read labels, arrows and marks; it can occasionally invent details, so the text description is always sent too). If the vision call fails before streaming anything, it retries text-only on the normal model. Groq's model list changes: check `GET https://api.groq.com/openai/v1/models`. `ChatMsg.image_png` carries it per provider format. The side panel is one of `practice | canvas` (localStorage `cprep.panel`). In dev, `window.__excal` exposes the Excalidraw API for UI checks. First lazy import of Excalidraw makes Vite re-optimize deps and reload the page once.

**Next:** Phase 3 — scheduler, tray, Windows notifications, autostart, spaced-repetition review cards.
