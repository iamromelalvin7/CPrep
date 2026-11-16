//! Voice: Kokoro text-to-speech through a long-running Python sidecar, and speech-to-text through
//! Groq Whisper.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};

use crate::config::Config;

pub const VOICES: &[(&str, &str)] = &[
    ("af_heart", "Heart (American, female)"),
    ("af_bella", "Bella (American, female)"),
    ("am_michael", "Michael (American, male)"),
    ("am_fenrir", "Fenrir (American, male)"),
    ("bf_emma", "Emma (British, female)"),
    ("bm_george", "George (British, male)"),
];

struct Proc {
    _child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

/// One Kokoro process, started on first use and kept warm. Requests are served one at a time.
pub struct Tts {
    python: PathBuf,
    script: PathBuf,
    proc: tokio::sync::Mutex<Option<Proc>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Speech {
    pub wav_b64: String,
    pub seconds: f64,
    pub ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct VoiceStatus {
    pub python: String,
    pub python_found: bool,
    pub script_found: bool,
    pub running: bool,
    pub stt_ready: bool,
    pub voices: Vec<(String, String)>,
}

pub fn default_python() -> PathBuf {
    if let Ok(p) = std::env::var("KOKORO_PYTHON") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    let base = PathBuf::from(home).join("kokoro-venv");
    if cfg!(windows) { base.join("Scripts").join("python.exe") } else { base.join("bin").join("python") }
}

impl Tts {
    pub fn new(python: PathBuf, script: PathBuf) -> Self {
        Tts { python, script, proc: tokio::sync::Mutex::new(None) }
    }

    pub async fn status(&self, cfg: &Config) -> VoiceStatus {
        VoiceStatus {
            python: self.python.to_string_lossy().to_string(),
            python_found: self.python.is_file(),
            script_found: self.script.is_file(),
            running: self.proc.lock().await.is_some(),
            stt_ready: cfg.groq_key.is_some(),
            voices: VOICES.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        }
    }

    async fn start(&self) -> Result<Proc> {
        if !self.python.is_file() {
            bail!(
                "Kokoro's Python wasn't found at {}. Set KOKORO_PYTHON in .env to the python.exe of your Kokoro venv.",
                self.python.display()
            );
        }
        let mut cmd = tokio::process::Command::new(&self.python);
        cmd.arg(&self.script)
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUNBUFFERED", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // no console window
        let mut child = cmd.spawn().context("couldn't start the Kokoro voice process")?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let mut lines = BufReader::new(child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?).lines();
        // Loading torch and the model takes a while on CPU the first time.
        let ready = tokio::time::timeout(Duration::from_secs(180), lines.next_line())
            .await
            .map_err(|_| anyhow!("Kokoro took too long to start"))??
            .ok_or_else(|| anyhow!("Kokoro exited while starting. Check that the venv still works: run test_kokoro.py"))?;
        let v: Value = serde_json::from_str(&ready).unwrap_or(Value::Null);
        if v["ready"] != true {
            bail!("Kokoro didn't start properly: {}", ready.chars().take(200).collect::<String>());
        }
        Ok(Proc { _child: child, stdin, lines, next_id: 1 })
    }

    /// Start the process now so the first sentence isn't slow.
    pub async fn warm_up(&self) -> Result<()> {
        let mut guard = self.proc.lock().await;
        if guard.is_none() {
            *guard = Some(self.start().await?);
        }
        Ok(())
    }

    pub async fn speak(&self, text: &str, voice: &str, speed: f64) -> Result<Speech> {
        let mut guard = self.proc.lock().await;
        if guard.is_none() {
            *guard = Some(self.start().await?);
        }
        let result = async {
            let p = guard.as_mut().unwrap();
            let id = p.next_id;
            p.next_id += 1;
            let req = json!({"id": id, "text": text, "voice": voice, "speed": speed.clamp(0.6, 1.6)});
            p.stdin.write_all(format!("{req}\n").as_bytes()).await?;
            p.stdin.flush().await?;
            let line = tokio::time::timeout(Duration::from_secs(120), p.lines.next_line())
                .await
                .map_err(|_| anyhow!("Kokoro took too long on one sentence"))??
                .ok_or_else(|| anyhow!("Kokoro stopped unexpectedly"))?;
            let v: Value = serde_json::from_str(&line)?;
            if let Some(e) = v["error"].as_str() {
                bail!("Kokoro: {e}");
            }
            Ok(Speech {
                wav_b64: v["wav_b64"].as_str().unwrap_or_default().to_string(),
                seconds: v["seconds"].as_f64().unwrap_or(0.0),
                ms: v["ms"].as_u64().unwrap_or(0),
            })
        }
        .await;
        // A dead process is replaced on the next request.
        if let Err(e) = &result {
            if !e.to_string().starts_with("Kokoro: ") {
                *guard = None;
            }
        }
        result
    }
}

// ---------- Speech-to-text (Groq Whisper) ----------

#[derive(Debug, Deserialize)]
struct WhisperReply {
    text: String,
}

pub async fn transcribe(cfg: &Config, audio_b64: &str, mime: &str) -> Result<String> {
    let key = cfg
        .groq_key
        .clone()
        .ok_or_else(|| anyhow!("Speech-to-text needs GROQ_API_KEY in your .env file."))?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(audio_b64).context("bad audio data")?;
    if bytes.len() > 24 * 1024 * 1024 {
        bail!("That recording is too long to transcribe in one go (Groq's limit is 25 MB). Keep answers under about 20 minutes.");
    }
    if bytes.len() < 1000 {
        bail!("The recording was empty. Check that your microphone is allowed and not muted.");
    }
    let ext = if mime.contains("ogg") {
        "ogg"
    } else if mime.contains("mp4") || mime.contains("m4a") {
        "m4a"
    } else if mime.contains("wav") {
        "wav"
    } else {
        "webm"
    };
    let part = reqwest::multipart::Part::bytes(bytes).file_name(format!("answer.{ext}")).mime_str(mime.split(';').next().unwrap_or("audio/webm"))?;
    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", std::env::var("GROQ_STT_MODEL").unwrap_or_else(|_| "whisper-large-v3-turbo".into()))
        .text("language", "en")
        .text("response_format", "json")
        // Biases Whisper toward the vocabulary of the subject.
        .text("prompt", "Software engineering interview and tutoring: Python, Flask, FastAPI, Django, SQL, Postgres, Redis, HTTP, REST, API, JSON, TCP, DNS, Big-O, hash map, recursion.");
    let resp = reqwest::Client::new()
        .post("https://api.groq.com/openai/v1/audio/transcriptions")
        .bearer_auth(key)
        .multipart(form)
        .timeout(Duration::from_secs(90))
        .send()
        .await
        .context("Couldn't reach Groq for speech-to-text")?;
    if !resp.status().is_success() {
        let status = resp.status();
        bail!("Groq speech-to-text returned {status}: {}", resp.text().await.unwrap_or_default().chars().take(300).collect::<String>());
    }
    let r: WhisperReply = resp.json().await?;
    Ok(r.text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_python_points_into_the_venv() {
        let p = default_python();
        assert!(p.to_string_lossy().contains("kokoro-venv") || std::env::var("KOKORO_PYTHON").is_ok());
    }
}
