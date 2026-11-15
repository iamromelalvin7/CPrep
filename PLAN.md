# CPrep (Career Prep): Plan

## Context

You're in Level 300 and graduate in 2028. You want one local desktop app that turns your own study folders into guided courses for the careers you're considering: AI engineering, backend/systems, networking, AI automation, DSA, system design, and interview prep. Hardware/embedded/robotics is paused. The app should:

- check what you already know, then skip it
- build a curriculum you can step back through and retake
- teach from your folders (RAG), with study notes, slides and docs
- draw on a canvas while it explains, and give you a canvas/editor where you do tasks and it grades them
- schedule sessions and remind you ("15 min to start")
- talk to you with Kokoro and listen to you through speech-to-text
- use Claude ($25 credit) for real teaching and free/local models for testing

All of it stays on your laptop.

**This turn only plans.** Once you approve, the first step creates `Desktop\programming\CPrep\` and puts this plan in it as `PLAN.md`. I used `CPrep` with no space because spaces in paths cause trouble with cargo, npm and Python tooling.

## Your machine (checked)

| | |
|---|---|
| CPU | AMD Ryzen 7 5700U, 8 cores / 16 threads |
| GPU | Radeon Vega iGPU, 512 MB shared, so treat inference as **CPU-only** |
| RAM | 15.4 GB usable |
| Disk | **only ~21 GB free on C:**. This is the real limit for local models. |
| Installed | Rust 1.99.1, Node 24.16, Python 3.11 + 3.14, Git, **Ollama** with `gemma2:9b` and `nomic-embed-text`, Kokoro venv (`~/kokoro-venv`) |

What the hardware means:
- 3–4B models at Q4 run at about 10–15 tok/s. Good enough for quick grading, quizzes and tagging.
- 7–9B models such as your gemma2:9b run at about 4–7 tok/s. Usable, but slow for live tutoring.
- Anything at 14B or above is too slow, and the disk can't hold several.
- So **Claude does the teaching. Local models do cheap background work.** One small model is enough: a ~4B instruct model (about 2.5 GB). Keep `nomic-embed-text` for embeddings. Consider deleting `gemma2:9b` later to free 5.4 GB.

## Language decision: Rust, but not Rust-only

Pure Rust UI toolkits (egui, iced, Slint) are fast. But they make three of your top requirements hard: a **beautiful UI**, an **AI-drawable canvas**, and a **real code editor**. The right stack is **Tauri 2**:

- **Rust backend (the core):** files, ingestion, SQLite + vector search, the scheduler, OS notifications, the system tray, running and grading your code, and calls to Ollama, Claude and Groq. Your Rust learning still happens here, on real systems work.
- **Web frontend inside the native window:** React + TypeScript + Vite, with Monaco (the VS Code editor), Excalidraw (canvas), Mermaid (diagrams), KaTeX (math), and Motion/GSAP for polish. This is how the app gets to look great.
- Tauri apps use about 10× less RAM than Electron, which matters because your local LLM needs the memory.
- The voice sidecar stays in **Python**, because Kokoro already works in `~/kokoro-venv`. Rust starts it as a child process.

**Python-only would be faster to build, and it isn't the better choice here.** It would mean dropping Rust, and Python UI options (PyQt, Flet) look worse. Tauri gives you both.

## Architecture

```
┌──────────────── React UI (Tauri webview) ───────────────────┐
│ Dashboard · Tracks · Lesson view · Canvas · Code lab ·      │
│ Notes/Slides · Schedule · Interview room · Settings         │
└──────────────────────── Tauri commands/events ──────────────┘
┌──────────────────────── Rust core ──────────────────────────┐
│ library/   folder connect, watch (notify), parse, chunk     │
│ rag/       embed via Ollama → SQLite + sqlite-vec, retrieve │
│ tutor/     LLM router (Claude | Ollama | Groq), prompts,    │
│            curriculum engine, placement tests, grading      │
│ runner/    run user code (python/node/java/sql) w/ timeout  │
│ sched/     sessions, reminders, spaced repetition (FSRS)    │
│ store/     SQLite (rusqlite) in %APPDATA%\CPrep             │
│ voice/     spawn Kokoro sidecar; STT (whisper)              │
└─────────────────────────────────────────────────────────────┘
```

**LLM router:** a single `LlmProvider` trait with three implementations:
- **Claude**, through raw HTTPS with `reqwest`. There is no official Rust SDK, so the app calls the Messages API directly, with streaming and prompt caching.
- **Ollama**, the local server at `localhost:11434`.
- **Groq/Gemini free tiers**, through their OpenAI-compatible APIs.

Settings decides which task goes to which provider. A "Test mode" switch sends everything to free models.

**Claude model and cost.** A "session" here means one ~45–60 min, ~30-message tutoring sitting, not one topic. A topic like FastAPI may take 5–10 sessions. Prices per million tokens (input/output): Opus 5.5 $4/$20, Sonnet 5.5 $2/$10, Haiku 5.5 $0.10/$0.50. Rough cost per session with the course context prompt-cached:

| Model | Cost per session | Sessions on $25 |
|---|---|---|
| Opus 5.5 | ~$0.65 | ~38 |
| Sonnet 5.5 | ~$0.33 | ~75 |
| Haiku 5.5 | ~$0.02 | ~1,000+ |

**Decision (2026-11-15): Haiku 5.5 is the default tutor.** Lessons are grounded in your own uploaded material through RAG, so the tutor mostly explains, quizzes and checks rather than reasoning from scratch. Haiku handles that well at about 1/40th of Opus's price. Sonnet 5.5 is reserved for the rare heavy jobs: generating a whole curriculum (once per track) and critiquing system-design diagrams or hard code. Opus 5.5 is an optional per-question "escalate" button. The expected mix is several hundred sessions on $25.

**Gemini** (Google AI Studio free tier) is a fourth provider, used for test mode and overflow. Its free-tier prompts may be used for training, which is fine for public study material but not for private notes.

**Secrets:** API keys live in a `.env` file at the project root (loaded with `dotenvy`). `.env` is gitignored, and only `.env.example` with empty values is committed. Keys are needed only when you actually use a cloud provider; test mode needs none.

Prices change, so recheck them before relying on these numbers. A live cost meter shows spend.

## Features → how each works

1. **Tracks (spaces).** Each track is isolated: its own connected folders, curriculum, notes and progress. Starting tracks: AI Engineering, Backend Engineering (Python: Flask/FastAPI/Django; Java: Spring Boot; JS: Node/Express unlocks later), Databases (SQL/Postgres), DSA, System Design, Networking, AI Automation, Interview Prep. Hardware is listed as "paused".
2. **Connect folders.** You pick folders on your Desktop and nothing is copied. A Rust watcher re-ingests changed files. Supported: PDF, MD, TXT, DOCX, PPTX (text), source code, and Jupyter notebooks. Files are chunked, embedded with `nomic-embed-text`, and stored with their source path and page so lessons can cite them ("from *Designing Data-Intensive Apps*, p. 112").
3. **Placement.** On entering a track, it asks what you know ("Do you know Python?"). If you say yes, it gives a 5–10 question adaptive quiz (MCQ + one small coding task). Modules you pass are marked "tested out", so you can skip them.
4. **Curriculum engine.** Claude builds a course from your goals, your placement results and what your folders contain: Phases → Modules → Lessons, each with objectives and a check. It's stored as data, so you can go back to any lesson, retake a phase, or ask it to regenerate a module.
5. **Lesson view.** A split screen: the tutor's chat/explanation streams on the left, and a canvas or code panel sits on the right. The tutor can write to the canvas through *tool calls*: it returns structured drawing commands (Excalidraw elements, Mermaid diagrams, SVG). The usual term is **"generative UI"**, or an AI whiteboard. Examples: a TCP handshake diagram, a B-tree, request flow through Django.
6. **Your canvas + Code lab.**
   - **Code:** Monaco editor. Ctrl+Enter runs your code locally (python, node, java, sqlite/postgres) with a timeout. The output plus AI-written test cases go back to the tutor, which grades the work and explains mistakes.
   - **Diagrams:** for system design, you draw in Excalidraw, and the canvas JSON (plus a screenshot) goes to Claude, which critiques it.
7. **Notes, slides, docs.** Each lesson creates Markdown study notes you can edit, saved as real `.md` files in `CPrep\notes\<track>\`. Slides are generated Markdown shown in-app as a deck, with PDF export.
8. **Scheduling + notifications.**
   - You plan sessions on a weekly calendar. A tray process keeps running, and Windows sends toasts at T-30, T-15 and T-0 ("Networking: Subnetting in 15 min").
   - Autostart at login.
   - Spaced repetition (FSRS) schedules review cards from what you've learned.
9. **Voice.**
   - Speaking: Kokoro TTS (`af_heart`, the same setup as `~/test_kokoro.py`), sentence-chunked so audio starts quickly.
   - Listening: STT with **local whisper.cpp** (`base.en`, ~150 MB, private), or **Groq `whisper-large-v3-turbo`** (free, faster, but the audio goes online).
   - This powers "explain it back to me" and mock interviews.
10. **Interview room.** Timed DSA problems in the Code lab, system design on the canvas, behavioral STAR practice by voice, and a scored debrief afterward.
11. **Progress dashboard.** Streaks, mastery per module, weak spots, upcoming sessions, and API spend.

## Starting course outline (seed; Claude adapts it per placement)

- **Foundations (shared):** Git, Linux CLI, HTTP, JSON, testing basics, Python fluency check.
- **Backend:** REST design → FastAPI → Flask → Django → auth (JWT/OAuth) → SQL & Postgres (joins, indexes, transactions, migrations) → caching (Redis) → queues → Docker → deploying → Spring Boot (Java) → Node/Express (after JS).
- **DSA:** complexity → arrays/strings → hashing → two pointers/sliding window → stacks/queues → linked lists → trees/BST → heaps → graphs (BFS/DFS, Dijkstra) → DP → interview patterns.
- **System design:** scalability, load balancing, caching, DB sharding/replication, CAP, queues/streams, rate limiting, design-X exercises.
- **Networking:** OSI/TCP-IP, IP/subnetting, TCP vs UDP, DNS, HTTP/TLS, routing basics, sockets in Python, troubleshooting tools.
- **AI engineering:** LLM APIs, prompting, embeddings & RAG, tool use/agents, evals, fine-tuning concepts, serving & cost.
- **AI automation:** workflows (n8n-style), webhooks, scraping/APIs, agentic automations for businesses, a client-project portfolio.
- **Interview prep:** DSA drills, system design mocks, behavioral, résumé/portfolio, remote-role applications.

## Design direction

- Calm, focused "study cockpit": dark-first, with a light theme too.
- Fonts: an expressive display serif for headings (e.g. *Fraunces* or *Instrument Serif*), *IBM Plex Sans* for body text, *JetBrains Mono* for code.
- Color: one restrained accent per track, hairline rules instead of card soup, and motion only where it means something (streaming text, canvas drawing, phase transitions).
- At build time I'll follow your global ruleset: frontend-design → design-taste → animate → web-design-guidelines, with Playwright screenshots at desktop and mobile widths.

## Build phases (honest timeline)

The full vision is weeks of work. Tonight gets a **working vertical slice**, not everything.

**Phase 0 (tonight): "It teaches me from my folder"**
1. Create `Desktop\programming\CPrep\`, copy this plan in as `PLAN.md`, add a `CLAUDE.md`, and run `git init`.
2. Scaffold Tauri 2 + React + TS + Vite. Install `motion gsap @gsap/react`.
3. Set up SQLite (rusqlite) with tables `tracks`, `folders`, `documents`, `chunks`, `curricula`, `lessons`, `progress`.
4. Connect a folder → parse MD/TXT/PDF → chunk → embed with Ollama → vector search.
5. LLM router: Claude (reqwest, streaming) + Ollama, with API keys read from a gitignored `.env` (`dotenvy`).
6. A placement quiz plus curriculum generation for one track, and a lesson chat streaming with citations.
7. Basic styled shell: sidebar of tracks, a lesson view, and settings.

**Phase 1:** Code lab: Monaco + local runner + AI grading. Notes saved as `.md`.
**Phase 2:** Canvas: Excalidraw + Mermaid. Tutor drawing tools. Diagram critique.
**Phase 3:** Scheduler, tray, Windows notifications, autostart. FSRS review cards.
**Phase 4:** Voice: Kokoro sidecar + whisper STT. Mock interview room.
**Phase 5:** Slides/PDF export, dashboard, cost meter, polish and design audit.

## Key crates and packages

- **Rust:** `tauri` 2, `tauri-plugin-notification`, `tauri-plugin-autostart`, `tauri-plugin-dialog`, `rusqlite` (bundled) + `sqlite-vec`, `reqwest` (stream), `tokio`, `notify`, `pdf-extract`, `docx-rs`, `serde`, `dotenvy`.
- **Frontend:** `react`, `@monaco-editor/react`, `@excalidraw/excalidraw`, `mermaid`, `katex`, `react-markdown`, `motion`, `gsap`.
- **Local models:** `ollama pull` one ~4B instruct model. Keep `nomic-embed-text`.

## Verification (each phase)

- `cargo test` covers the chunker, the retriever, the router (with mocked HTTP) and the scheduler time math.
- Test mode (Ollama only) runs the whole flow end to end without spending Claude credit: connect a sample folder → placement → curriculum → a 3-turn lesson that cites a real file.
- Playwright against the Vite dev server (port 5173) at 1440×900 and 390×844: zero console errors, no overflow, animations settle. Screenshots go in `.playwright-artifacts/`.
- Manual: set a session 2 minutes ahead and check that the Windows toast fires; run a deliberately wrong solution and check that the grader catches it.

## Open choices (defaults chosen, change any)

- Folder name `CPrep` (no space).
- STT: local whisper.cpp by default, with Groq as the fast option.
- Tutoring model: Haiku 5.5 by default; Sonnet 5.5 for curriculum design and critiques; Opus 5.5 on demand.
- Which track to build first tonight: **Backend** is suggested, since it uses the most of what you already know.
