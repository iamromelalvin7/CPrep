# CPrep — Career Prep

A local-first desktop AI tutor that turns your own study folders into guided courses for software careers: backend, AI engineering, networking, AI automation, DSA, system design and interview prep.

**Status:** Phase 0 working: connect study folders, take a placement quiz, get a generated course, and learn in a streaming lesson that cites your own files. See [PLAN.md](PLAN.md) for the roadmap.

## Stack (planned)
- **Tauri 2**: Rust core (ingestion, SQLite + vector search, scheduler, notifications, code runner, LLM router)
- **React + TypeScript + Vite** UI (Monaco editor, Excalidraw canvas, Mermaid)
- **Ollama** for local models, **Claude** for tutoring, **Gemini/Groq** free tiers for testing
- **Kokoro** TTS + Whisper STT for voice

## Setup
Needs Rust, Node 20+, and [Ollama](https://ollama.com) with a chat model and `nomic-embed-text`:
```bash
ollama pull qwen3:4b && ollama pull nomic-embed-text
cp .env.example .env   # add only the keys you use; test mode needs none
npm install
npm run tauri dev
```
Tests: `cd src-tauri && cargo test` (unit) and `cargo test --test e2e -- --ignored --nocapture` (full flow on a real folder, local models).
Everything, including your data, stays on your machine.
