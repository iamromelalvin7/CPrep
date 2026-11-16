# CPrep — Career Prep

A local-first desktop AI tutor that turns your own study folders into guided courses for software careers: backend, AI engineering, networking, AI automation, DSA, system design and interview prep.

**Status:** all planned phases built: folder ingestion and RAG, placement and course design, streaming lessons with cited sources, code lab with local grading, canvas with diagrams and drawing critique, schedule with Windows reminders and tray, spaced-repetition review cards, voice (Kokoro + Whisper), mock interviews with scored debriefs, a Today dashboard, and notes as slides or PDF. See [PLAN.md](PLAN.md) for the original design.

## Stack (planned)
- **Tauri 2**: Rust core (ingestion, SQLite + vector search, scheduler, notifications, code runner, LLM router)
- **React + TypeScript + Vite** UI (Monaco editor, Excalidraw canvas, Mermaid)
- **Ollama** for local models, **Claude** for tutoring, **Gemini/Groq** free tiers for testing
- **Kokoro** TTS + Whisper STT for voice

## Install (Windows)
Run the installer from `src-tauri/target/release/bundle/nsis/` (or the `.msi` in `bundle/msi/`). The installed app reads its settings from `%APPDATA%\com.romel.cprep\.env`: copy your `.env` there.

## Setup (development)
Needs Rust, Node 20+, and [Ollama](https://ollama.com) with a chat model and `nomic-embed-text`:
```bash
ollama pull qwen3:4b && ollama pull nomic-embed-text
cp .env.example .env   # add only the keys you use; test mode needs none
npm install
npm run tauri dev
```
Tests: `cd src-tauri && cargo test` (unit) and `cargo test --test e2e -- --ignored --nocapture` (full flow on a real folder, local models).
Everything, including your data, stays on your machine.
