# CPrep — Career Prep

A local-first desktop AI tutor that turns your own study folders into guided courses for software careers: backend, AI engineering, networking, AI automation, DSA, system design and interview prep.

**Status:** planning. See [PLAN.md](PLAN.md) for the full design and build phases.

## Stack (planned)
- **Tauri 2**: Rust core (ingestion, SQLite + vector search, scheduler, notifications, code runner, LLM router)
- **React + TypeScript + Vite** UI (Monaco editor, Excalidraw canvas, Mermaid)
- **Ollama** for local models, **Claude** for tutoring, **Gemini/Groq** free tiers for testing
- **Kokoro** TTS + Whisper STT for voice

## Setup
```bash
cp .env.example .env   # add only the keys you use
```
Everything, including your data, stays on your machine.
