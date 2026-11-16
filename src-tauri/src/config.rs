//! Settings from `.env`. Keys are never written anywhere by the app.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub anthropic_key: Option<String>,
    pub gemini_key: Option<String>,
    pub groq_key: Option<String>,
    pub ollama_host: String,
    pub ollama_chat_model: String,
    pub ollama_embed_model: String,
    pub tutor_model: String,
    pub curriculum_model: String,
    pub escalate_model: String,
    /// "local" (Ollama) or "gemini": which free provider test mode uses.
    pub test_provider: String,
    /// Groq model for requests with an image (canvas feedback).
    pub groq_vision_model: String,
    pub env_path: Option<PathBuf>,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// Look for `.env` in the working directory and its parents (dev runs from `src-tauri`),
/// then next to the executable (installed builds).
fn find_env() -> Option<PathBuf> {
    let mut starts: Vec<PathBuf> = vec![];
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            starts.push(dir.to_path_buf());
        }
    }
    for start in starts {
        for dir in start.ancestors().take(4) {
            let candidate = dir.join(".env");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

impl Config {
    pub fn load() -> Self {
        let env_path = find_env();
        if let Some(p) = &env_path {
            let _ = dotenvy::from_path_override(p);
        }
        Config {
            anthropic_key: var("ANTHROPIC_API_KEY"),
            gemini_key: var("GEMINI_API_KEY"),
            groq_key: var("GROQ_API_KEY"),
            ollama_host: var("OLLAMA_HOST").unwrap_or_else(|| "http://localhost:11434".into()),
            ollama_chat_model: var("OLLAMA_CHAT_MODEL").unwrap_or_else(|| "qwen3:4b".into()),
            ollama_embed_model: var("OLLAMA_EMBED_MODEL").unwrap_or_else(|| "nomic-embed-text".into()),
            tutor_model: var("TUTOR_MODEL").unwrap_or_else(|| "claude-haiku-5-5".into()),
            curriculum_model: var("CURRICULUM_MODEL").unwrap_or_else(|| "claude-sonnet-5-5".into()),
            escalate_model: var("ESCALATE_MODEL").unwrap_or_else(|| "claude-opus-5-5".into()),
            test_provider: var("TEST_PROVIDER").unwrap_or_else(|| "local".into()),
            groq_vision_model: var("GROQ_VISION_MODEL").unwrap_or_else(|| crate::llm::DEFAULT_GROQ_VISION_MODEL.into()),
            env_path,
        }
    }
}
