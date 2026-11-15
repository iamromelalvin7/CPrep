# CPrep: project context

**Goal:** Personal local-first AI tutor desktop app for career prep (backend, AI eng, networking, AI automation, DSA, system design, interviews). Full design: `PLAN.md`.

**Current state (2026-11-15):** Planning only. No code yet. Repo created on GitHub (public). Waiting for the user to say "start Phase 0".

**Decisions:**
- Stack: Tauri 2 (Rust core) + React/TS/Vite UI; Python Kokoro sidecar.
- Tutor model: Claude Haiku 5.5 by default; Sonnet 5.5 for curriculum + critiques; Opus 5.5 on demand. Gemini/Groq/Ollama for test mode.
- Secrets in `.env` (gitignored); `.env.example` is committed.
- Local model: `qwen3:4b` + `nomic-embed-text` (Ollama). CPU-only laptop (Ryzen 7 5700U, 16 GB, ~21 GB free disk).
- First test folder: `~/Desktop/programming/BACKEND JOURNEY` (34 items). First track: Backend.

**Next:** Phase 0: scaffold, folder ingestion, LLM router, placement + curriculum, streaming lesson chat.
