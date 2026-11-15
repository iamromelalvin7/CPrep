//! LLM router: Claude (Messages API over raw HTTPS; there is no official Rust SDK),
//! Ollama (local), and OpenAI-compatible free tiers (Gemini, Groq).

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::Config;

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2026-10-21";
const GEMINI_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/openai";
const GROQ_BASE: &str = "https://api.groq.com/openai/v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMsg {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Usage {
    pub provider: String,
    pub model: String,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
}

impl Usage {
    /// USD per million tokens (input, output). Prices change: recheck before trusting totals.
    fn prices(model: &str) -> (f64, f64) {
        if model.starts_with("claude-haiku-5") {
            (0.10, 0.50)
        } else if model.starts_with("claude-sonnet") {
            (2.0, 10.0)
        } else if model.starts_with("claude-opus-5-5") {
            (4.0, 20.0)
        } else if model.starts_with("claude-opus") {
            (5.0, 25.0)
        } else if model.starts_with("claude-fable") {
            (8.0, 40.0)
        } else {
            (0.0, 0.0) // local and free tiers
        }
    }

    pub fn cost_usd(&self) -> f64 {
        let (i, o) = Self::prices(&self.model);
        (self.input as f64 * i
            + self.cache_write as f64 * i * 1.25
            + self.cache_read as f64 * i * 0.1
            + self.output as f64 * o)
            / 1_000_000.0
    }
}

#[derive(Clone, Debug)]
pub enum Provider {
    Claude { key: String, model: String },
    Ollama { host: String, model: String },
    OpenAiCompat { name: &'static str, base: &'static str, key: String, model: String },
}

/// What a request is for. Routing differs per task in live mode.
#[derive(Clone, Copy, Debug)]
pub enum Task {
    Tutor,
    Curriculum,
    Quiz,
    Escalate,
}

/// Pick a provider. `live == false` is test mode: never spends Claude credit.
pub fn route(cfg: &Config, live: bool, task: Task) -> Result<Provider> {
    if live {
        let key = cfg
            .anthropic_key
            .clone()
            .ok_or_else(|| anyhow!("Live mode needs ANTHROPIC_API_KEY in your .env file."))?;
        let model = match task {
            Task::Tutor | Task::Quiz => cfg.tutor_model.clone(),
            Task::Curriculum => cfg.curriculum_model.clone(),
            Task::Escalate => cfg.escalate_model.clone(),
        };
        return Ok(Provider::Claude { key, model });
    }
    match cfg.test_provider.as_str() {
        "gemini" => {
            let key = cfg
                .gemini_key
                .clone()
                .ok_or_else(|| anyhow!("TEST_PROVIDER=gemini needs GEMINI_API_KEY in .env."))?;
            Ok(Provider::OpenAiCompat {
                name: "gemini",
                base: GEMINI_BASE,
                key,
                model: std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-2.5-flash".into()),
            })
        }
        "groq" => {
            let key = cfg
                .groq_key
                .clone()
                .ok_or_else(|| anyhow!("TEST_PROVIDER=groq needs GROQ_API_KEY in .env."))?;
            Ok(Provider::OpenAiCompat {
                name: "groq",
                base: GROQ_BASE,
                key,
                model: std::env::var("GROQ_MODEL").unwrap_or_else(|_| "openai/gpt-oss-120b".into()),
            })
        }
        _ => Ok(Provider::Ollama { host: cfg.ollama_host.clone(), model: cfg.ollama_chat_model.clone() }),
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .expect("http client")
}

async fn check(resp: reqwest::Response, who: &str) -> Result<reqwest::Response> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    bail!("{who} returned {status}: {}", body.chars().take(600).collect::<String>())
}

/// Small models sometimes wrap JSON in prose, fences or <think> blocks.
pub fn extract_json(text: &str) -> Result<Value> {
    let cleaned = strip_think(text);
    if let Ok(v) = serde_json::from_str(cleaned.trim()) {
        return Ok(v);
    }
    let start = cleaned.find(|c| c == '{' || c == '[').ok_or_else(|| anyhow!("no JSON in model output"))?;
    let end = cleaned.rfind(|c| c == '}' || c == ']').ok_or_else(|| anyhow!("no JSON in model output"))?;
    serde_json::from_str(&cleaned[start..=end]).context("model returned malformed JSON")
}

pub fn strip_think(text: &str) -> String {
    let mut out = text.to_string();
    while let (Some(s), Some(e)) = (out.find("<think>"), out.find("</think>")) {
        if e < s {
            break;
        }
        out.replace_range(s..e + "</think>".len(), "");
    }
    out
}

impl Provider {
    pub fn label(&self) -> String {
        match self {
            Provider::Claude { model, .. } => format!("Claude · {model}"),
            Provider::Ollama { model, .. } => format!("Local · {model}"),
            Provider::OpenAiCompat { name, model, .. } => format!("{name} · {model}"),
        }
    }

    /// Label naming the model that actually answered (Ollama may fall back to another installed model).
    pub fn label_for(&self, usage: &Usage) -> String {
        match self {
            Provider::Ollama { .. } if !usage.model.is_empty() => format!("Local · {}", usage.model),
            _ => self.label(),
        }
    }

    /// One request that must come back as JSON matching `schema`.
    pub async fn complete_json(&self, system: &str, user: &str, schema: Value) -> Result<(Value, Usage)> {
        match self {
            Provider::Claude { key, model } => {
                let body = json!({
                    "model": model,
                    "max_tokens": 16000,
                    "system": system,
                    "messages": [{"role": "user", "content": user}],
                    "output_config": {"format": {"type": "json_schema", "schema": schema}},
                });
                let resp = client()
                    .post(ANTHROPIC_URL)
                    .header("x-api-key", key)
                    .header("anthropic-version", ANTHROPIC_VERSION)
                    .json(&body)
                    .send()
                    .await?;
                let v: Value = check(resp, "Claude").await?.json().await?;
                match v["stop_reason"].as_str() {
                    Some("refusal") => bail!("Claude declined this request."),
                    Some("max_tokens") => bail!("Claude ran out of output tokens."),
                    _ => {}
                }
                let text = v["content"]
                    .as_array()
                    .and_then(|blocks| blocks.iter().find(|b| b["type"] == "text"))
                    .and_then(|b| b["text"].as_str())
                    .ok_or_else(|| anyhow!("Claude response had no text"))?;
                Ok((extract_json(text)?, claude_usage(model, &v["usage"])))
            }
            Provider::Ollama { host, model } => {
                let model = resolve_ollama_model(host, model).await?;
                let body = json!({
                    "model": model,
                    "stream": false,
                    "format": schema,
                    "think": false,
                    "options": {"num_ctx": 8192, "temperature": 0.3},
                    "messages": [
                        {"role": "system", "content": system},
                        {"role": "user", "content": user}
                    ],
                });
                let v: Value = ollama_post(host, body).await?.json().await?;
                let text = v["message"]["content"].as_str().unwrap_or_default();
                let usage = Usage {
                    provider: "ollama".into(),
                    model: model.clone(),
                    input: v["prompt_eval_count"].as_i64().unwrap_or(0),
                    output: v["eval_count"].as_i64().unwrap_or(0),
                    ..Default::default()
                };
                Ok((extract_json(text)?, usage))
            }
            Provider::OpenAiCompat { name, base, key, model } => {
                let body = json!({
                    "model": model,
                    "messages": [
                        {"role": "system", "content": system},
                        {"role": "user", "content": user}
                    ],
                    "response_format": {"type": "json_schema", "json_schema": {"name": "output", "schema": schema}},
                });
                let resp = client()
                    .post(format!("{base}/chat/completions"))
                    .bearer_auth(key)
                    .json(&body)
                    .send()
                    .await?;
                let v: Value = check(resp, name).await?.json().await?;
                let text = v["choices"][0]["message"]["content"].as_str().unwrap_or_default();
                let usage = Usage {
                    provider: name.to_string(),
                    model: model.clone(),
                    input: v["usage"]["prompt_tokens"].as_i64().unwrap_or(0),
                    output: v["usage"]["completion_tokens"].as_i64().unwrap_or(0),
                    ..Default::default()
                };
                Ok((extract_json(text)?, usage))
            }
        }
    }

    /// Streamed chat. `on_delta` gets each text fragment as it arrives.
    pub async fn stream_chat(
        &self,
        system: &str,
        messages: &[ChatMsg],
        mut on_delta: impl FnMut(&str),
    ) -> Result<(String, Usage)> {
        let mut full = String::new();
        match self {
            Provider::Claude { key, model } => {
                let body = json!({
                    "model": model,
                    "max_tokens": 8000,
                    "stream": true,
                    // Auto-cache the conversation prefix; system prompt is stable per lesson.
                    "cache_control": {"type": "ephemeral"},
                    "system": system,
                    "output_config": {"effort": "low"},
                    "messages": messages,
                });
                let resp = client()
                    .post(ANTHROPIC_URL)
                    .header("x-api-key", key)
                    .header("anthropic-version", ANTHROPIC_VERSION)
                    .json(&body)
                    .send()
                    .await?;
                let resp = check(resp, "Claude").await?;
                let mut usage = Usage { provider: "anthropic".into(), model: model.clone(), ..Default::default() };
                let mut stop_reason = String::new();
                for_each_sse(resp, |data| {
                    let v: Value = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(_) => return Ok(()),
                    };
                    match v["type"].as_str() {
                        Some("message_start") => {
                            let u = claude_usage(model, &v["message"]["usage"]);
                            usage.input = u.input;
                            usage.cache_read = u.cache_read;
                            usage.cache_write = u.cache_write;
                        }
                        Some("content_block_delta") if v["delta"]["type"] == "text_delta" => {
                            let t = v["delta"]["text"].as_str().unwrap_or_default();
                            full.push_str(t);
                            on_delta(t);
                        }
                        Some("message_delta") => {
                            usage.output = v["usage"]["output_tokens"].as_i64().unwrap_or(usage.output);
                            if let Some(r) = v["delta"]["stop_reason"].as_str() {
                                stop_reason = r.to_string();
                            }
                        }
                        Some("error") => bail!("Claude stream error: {}", v["error"]["message"]),
                        _ => {}
                    }
                    Ok(())
                })
                .await?;
                if stop_reason == "refusal" && full.is_empty() {
                    bail!("Claude declined to answer that.");
                }
                Ok((full, usage))
            }
            Provider::Ollama { host, model } => {
                let model = resolve_ollama_model(host, model).await?;
                let mut msgs = vec![json!({"role": "system", "content": system})];
                msgs.extend(messages.iter().map(|m| json!({"role": m.role, "content": m.content})));
                let body = json!({
                    "model": model,
                    "stream": true,
                    "think": false,
                    "options": {"num_ctx": 8192, "temperature": 0.5},
                    "messages": msgs,
                });
                let resp = ollama_post(host, body).await?;
                let mut usage = Usage { provider: "ollama".into(), model: model.clone(), ..Default::default() };
                let mut stream = resp.bytes_stream();
                let mut buf = Vec::new();
                while let Some(chunk) = stream.next().await {
                    buf.extend_from_slice(&chunk?);
                    while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                        let line: Vec<u8> = buf.drain(..=pos).collect();
                        let Ok(v) = serde_json::from_slice::<Value>(&line) else { continue };
                        if let Some(err) = v["error"].as_str() {
                            bail!("Ollama: {err}");
                        }
                        if let Some(t) = v["message"]["content"].as_str() {
                            if !t.is_empty() {
                                full.push_str(t);
                                on_delta(t);
                            }
                        }
                        if v["done"] == true {
                            usage.input = v["prompt_eval_count"].as_i64().unwrap_or(0);
                            usage.output = v["eval_count"].as_i64().unwrap_or(0);
                        }
                    }
                }
                Ok((strip_think(&full), usage))
            }
            Provider::OpenAiCompat { name, base, key, model } => {
                let mut msgs = vec![json!({"role": "system", "content": system})];
                msgs.extend(messages.iter().map(|m| json!({"role": m.role, "content": m.content})));
                let body = json!({"model": model, "stream": true, "messages": msgs});
                let resp = client()
                    .post(format!("{base}/chat/completions"))
                    .bearer_auth(key)
                    .json(&body)
                    .send()
                    .await?;
                let resp = check(resp, name).await?;
                let usage = Usage { provider: name.to_string(), model: model.clone(), ..Default::default() };
                for_each_sse(resp, |data| {
                    if data == "[DONE]" {
                        return Ok(());
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(data) {
                        if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
                            full.push_str(t);
                            on_delta(t);
                        }
                    }
                    Ok(())
                })
                .await?;
                Ok((full, usage))
            }
        }
    }
}

fn claude_usage(model: &str, u: &Value) -> Usage {
    Usage {
        provider: "anthropic".into(),
        model: model.to_string(),
        input: u["input_tokens"].as_i64().unwrap_or(0),
        output: u["output_tokens"].as_i64().unwrap_or(0),
        cache_read: u["cache_read_input_tokens"].as_i64().unwrap_or(0),
        cache_write: u["cache_creation_input_tokens"].as_i64().unwrap_or(0),
    }
}

/// Parse a Server-Sent Events body, calling `f` with each `data:` payload.
async fn for_each_sse(resp: reqwest::Response, mut f: impl FnMut(&str) -> Result<()>) -> Result<()> {
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    while let Some(chunk) = stream.next().await {
        buf.push_str(&String::from_utf8_lossy(&chunk?).replace("\r\n", "\n"));
        while let Some(pos) = buf.find("\n\n") {
            let event: String = buf.drain(..pos + 2).collect();
            for line in event.lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    f(data.trim())?;
                }
            }
        }
    }
    Ok(())
}

/// POST to Ollama's /api/chat. Older or non-thinking models reject `think`, so retry without it.
async fn ollama_post(host: &str, mut body: Value) -> Result<reqwest::Response> {
    let url = format!("{host}/api/chat");
    let resp = client()
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("Can't reach Ollama. Is it running? (start the Ollama app)")?;
    if resp.status() == reqwest::StatusCode::BAD_REQUEST {
        let text = resp.text().await.unwrap_or_default();
        if text.contains("think") {
            body.as_object_mut().unwrap().remove("think");
            let resp = client().post(&url).json(&body).send().await?;
            return check(resp, "Ollama").await;
        }
        bail!("Ollama returned 400: {text}");
    }
    check(resp, "Ollama").await
}

/// Use the configured model if installed, else the first installed chat model.
pub async fn resolve_ollama_model(host: &str, wanted: &str) -> Result<String> {
    let models = ollama_models(host).await?;
    if models.iter().any(|m| m == wanted || m.trim_end_matches(":latest") == wanted) {
        return Ok(wanted.to_string());
    }
    models
        .into_iter()
        .find(|m| !m.contains("embed"))
        .ok_or_else(|| anyhow!("No local chat model installed. Run: ollama pull {wanted}"))
}

pub async fn ollama_models(host: &str) -> Result<Vec<String>> {
    let v: Value = client()
        .get(format!("{host}/api/tags"))
        .send()
        .await
        .context("Can't reach Ollama. Is it running?")?
        .json()
        .await?;
    Ok(v["models"]
        .as_array()
        .map(|a| a.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect())
        .unwrap_or_default())
}

/// Embed texts with Ollama (nomic-embed-text wants task prefixes).
pub async fn embed(cfg: &Config, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
    let v: Value = check(
        client()
            .post(format!("{}/api/embed", cfg.ollama_host))
            .json(&json!({"model": cfg.ollama_embed_model, "input": inputs}))
            .send()
            .await
            .context("Can't reach Ollama for embeddings. Is it running?")?,
        "Ollama embed",
    )
    .await?
    .json()
    .await?;
    let arr = v["embeddings"].as_array().ok_or_else(|| anyhow!("no embeddings in response"))?;
    Ok(arr
        .iter()
        .map(|e| e.as_array().map(|x| x.iter().map(|f| f.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_from_noisy_output() {
        let v = extract_json("<think>hmm</think>Sure! ```json\n{\"a\": [1,2]}\n```").unwrap();
        assert_eq!(v["a"][1], 2);
    }

    #[test]
    fn haiku_cost_is_cheap() {
        let u = Usage { model: "claude-haiku-5-5".into(), input: 1_000_000, output: 1_000_000, ..Default::default() };
        assert!((u.cost_usd() - 0.60).abs() < 1e-9);
        let local = Usage { model: "qwen3:4b".into(), input: 5000, ..Default::default() };
        assert_eq!(local.cost_usd(), 0.0);
    }
}
