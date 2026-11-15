//! Folder ingestion: walk → extract text → chunk → embed → store.
//! Files stay where they are; only extracted text and embeddings go in the database.

use anyhow::Result;
use regex::Regex;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use walkdir::WalkDir;

use crate::config::Config;
use crate::{llm, rag};

const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "venv", ".venv", "env", "__pycache__", ".pytest_cache", ".mypy_cache",
    "dist", "build", "target", ".idea", ".vscode", "instance", "site-packages",
];
const CODE_EXT: &[&str] = &[
    "py", "js", "ts", "tsx", "jsx", "java", "rs", "go", "sql", "sh", "c", "cpp", "h", "cs", "rb", "php",
    "yml", "yaml", "toml", "html", "css",
];
const TEXT_EXT: &[&str] = &["md", "txt", "rst"];
const MAX_TEXT_BYTES: u64 = 2_000_000;
const MAX_PDF_BYTES: u64 = 80_000_000;
const CHUNK_CHARS: usize = 1400;
const CODE_CHUNK_LINES: usize = 60;
const EMBED_BATCH: usize = 16;

#[derive(Clone, Debug, Serialize)]
pub struct Progress {
    pub folder_id: i64,
    pub stage: String,
    pub done: usize,
    pub total: usize,
    pub current: String,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct Report {
    pub scanned: usize,
    pub updated: usize,
    pub removed: usize,
    pub chunks_embedded: usize,
    pub failed: Vec<String>,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Kind {
    Pdf,
    Text,
    Code,
    Notebook,
}

impl Kind {
    fn as_str(&self) -> &'static str {
        match self {
            Kind::Pdf => "pdf",
            Kind::Text => "text",
            Kind::Code => "code",
            Kind::Notebook => "notebook",
        }
    }
}

/// Decide whether a file is study material. Secrets, databases, logs and binaries are skipped.
pub fn classify(path: &Path) -> Option<Kind> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    if name.starts_with(".env") || name == ".flaskenv" || name.contains("secret") || name.ends_with(".pem") {
        return None;
    }
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    match ext.as_str() {
        "pdf" => Some(Kind::Pdf),
        "ipynb" => Some(Kind::Notebook),
        e if TEXT_EXT.contains(&e) => Some(Kind::Text),
        e if CODE_EXT.contains(&e) => Some(Kind::Code),
        _ => None,
    }
}

fn walk(root: &Path) -> Vec<(PathBuf, Kind, i64)> {
    WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            !(e.file_type().is_dir() && SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let kind = classify(e.path())?;
            let meta = e.metadata().ok()?;
            let limit = if kind == Kind::Pdf { MAX_PDF_BYTES } else { MAX_TEXT_BYTES };
            if meta.len() == 0 || meta.len() > limit {
                return None;
            }
            let mtime = meta
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs() as i64;
            Some((e.path().to_path_buf(), kind, mtime))
        })
        .collect()
}

static SECRET_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)((?:api[_-]?key|secret|password|passwd|token)\w*\s*[:=]\s*)(['"])[^'"\n]{6,}(['"])"#).unwrap()
});

/// Hard-coded credentials in study code should never reach an LLM.
pub fn redact(text: &str) -> String {
    SECRET_RE.replace_all(text, "$1$2[REDACTED]$3").into_owned()
}

/// Returns (page, text) sections. Page is Some for PDFs.
fn extract(path: &Path, kind: &Kind) -> Result<Vec<(Option<i64>, String)>> {
    match kind {
        Kind::Pdf => {
            let p = path.to_path_buf();
            // pdf-extract can panic on unusual PDFs; contain it.
            let pages = std::panic::catch_unwind(move || pdf_extract::extract_text_by_pages(&p))
                .map_err(|_| anyhow::anyhow!("PDF parser crashed"))??;
            Ok(pages
                .into_iter()
                .enumerate()
                .map(|(i, t)| (Some(i as i64 + 1), t))
                .filter(|(_, t)| !t.trim().is_empty())
                .collect())
        }
        Kind::Notebook => {
            let raw = std::fs::read_to_string(path)?;
            let v: serde_json::Value = serde_json::from_str(&raw)?;
            let mut out = String::new();
            for cell in v["cells"].as_array().into_iter().flatten() {
                let src = match &cell["source"] {
                    serde_json::Value::Array(lines) => lines.iter().filter_map(|l| l.as_str()).collect::<String>(),
                    serde_json::Value::String(s) => s.clone(),
                    _ => continue,
                };
                out.push_str(&src);
                out.push_str("\n\n");
            }
            Ok(vec![(None, redact(&out))])
        }
        Kind::Text | Kind::Code => {
            let bytes = std::fs::read(path)?;
            Ok(vec![(None, redact(&String::from_utf8_lossy(&bytes)))])
        }
    }
}

fn clean(text: &str) -> String {
    // Collapse runs of blank lines and trailing spaces that PDF extraction leaves behind.
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        let l = line.trim_end();
        if l.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(l);
        out.push('\n');
    }
    out.trim().to_string()
}

/// Prose chunking: pack paragraphs up to ~CHUNK_CHARS, carrying the last paragraph over for context.
pub fn chunk_prose(text: &str) -> Vec<String> {
    let text = clean(text);
    let mut chunks = Vec::new();
    let mut cur = String::new();
    let mut last_para = String::new();
    for para in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        // Split giant paragraphs on sentence-ish boundaries.
        let pieces: Vec<String> = if para.len() > CHUNK_CHARS {
            let mut v = Vec::new();
            let mut s = String::new();
            for word in para.split_whitespace() {
                if s.len() + word.len() > CHUNK_CHARS {
                    v.push(std::mem::take(&mut s));
                }
                if !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(word);
            }
            if !s.is_empty() {
                v.push(s);
            }
            v
        } else {
            vec![para.to_string()]
        };
        for piece in pieces {
            if !cur.is_empty() && cur.len() + piece.len() > CHUNK_CHARS {
                chunks.push(std::mem::take(&mut cur));
                if last_para.len() < CHUNK_CHARS / 4 {
                    cur.push_str(&last_para);
                    cur.push_str("\n\n");
                }
            }
            cur.push_str(&piece);
            cur.push_str("\n\n");
            last_para = piece;
        }
    }
    if !cur.trim().is_empty() {
        chunks.push(cur.trim().to_string());
    }
    chunks.into_iter().map(|c| c.trim().to_string()).filter(|c| c.len() > 40).collect()
}

/// Code chunking: fixed line windows with a small overlap.
pub fn chunk_code(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let end = (i + CODE_CHUNK_LINES).min(lines.len());
        let c = lines[i..end].join("\n");
        if c.trim().len() > 20 {
            out.push(c);
        }
        if end == lines.len() {
            break;
        }
        i = end - 5;
    }
    out
}

fn title_for(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

pub async fn ingest_folder(
    cfg: &Config,
    db: &Mutex<Connection>,
    folder_id: i64,
    root: &Path,
    progress: impl Fn(Progress),
) -> Result<Report> {
    let emit = |stage: &str, done, total, current: &str| {
        progress(Progress { folder_id, stage: stage.into(), done, total, current: current.into() })
    };
    emit("scanning", 0, 0, "");
    let files = {
        let root = root.to_path_buf();
        tokio::task::spawn_blocking(move || walk(&root)).await?
    };
    let mut report = Report { scanned: files.len(), ..Default::default() };

    // Diff against what we already have.
    let existing: HashMap<String, (i64, i64)> = {
        let conn = db.lock().unwrap();
        let mut stmt = conn.prepare("SELECT path, id, mtime FROM documents WHERE folder_id = ?1")?;
        let rows = stmt.query_map([folder_id], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?))))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    let seen: std::collections::HashSet<String> = files.iter().map(|(p, _, _)| p.to_string_lossy().to_string()).collect();
    {
        let conn = db.lock().unwrap();
        for (path, (id, _)) in &existing {
            if !seen.contains(path) {
                conn.execute("DELETE FROM documents WHERE id = ?1", [id])?;
                report.removed += 1;
            }
        }
    }
    let todo: Vec<_> = files
        .into_iter()
        .filter(|(p, _, m)| existing.get(&p.to_string_lossy().to_string()).map(|(_, old)| old != m).unwrap_or(true))
        .collect();

    let total = todo.len();
    for (i, (path, kind, mtime)) in todo.into_iter().enumerate() {
        let title = title_for(root, &path);
        emit("reading", i, total, &title);
        let sections = {
            let (p, k) = (path.clone(), kind.clone());
            tokio::task::spawn_blocking(move || extract(&p, &k)).await?
        };
        let sections = match sections {
            Ok(s) => s,
            Err(e) => {
                report.failed.push(format!("{title}: {e}"));
                continue;
            }
        };
        let mut chunks: Vec<(Option<i64>, String)> = Vec::new();
        for (page, text) in sections {
            let parts = if kind == Kind::Code { chunk_code(&text) } else { chunk_prose(&text) };
            chunks.extend(parts.into_iter().map(|c| (page, c)));
        }
        let conn = db.lock().unwrap();
        let path_s = path.to_string_lossy().to_string();
        conn.execute("DELETE FROM documents WHERE path = ?1", [&path_s])?;
        conn.execute(
            "INSERT INTO documents (folder_id, path, title, kind, mtime) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![folder_id, path_s, title, kind.as_str(), mtime],
        )?;
        let doc_id = conn.last_insert_rowid();
        let tx = conn.unchecked_transaction()?;
        for (ord, (page, text)) in chunks.iter().enumerate() {
            // Prefix the file name so the chunk carries its own context.
            let text = format!("[{title}]\n{text}");
            tx.execute(
                "INSERT INTO chunks (doc_id, ord, page, text) VALUES (?1, ?2, ?3, ?4)",
                params![doc_id, ord as i64, page, text],
            )?;
        }
        tx.commit()?;
        report.updated += 1;
    }

    // Embed everything in this folder that has no vector yet (also resumes interrupted runs).
    let pending: Vec<(i64, String)> = {
        let conn = db.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT c.id, c.text FROM chunks c JOIN documents d ON d.id = c.doc_id
             WHERE d.folder_id = ?1 AND c.embedding IS NULL ORDER BY c.id",
        )?;
        let rows = stmt.query_map([folder_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    let total = pending.len();
    for (b, batch) in pending.chunks(EMBED_BATCH).enumerate() {
        emit("embedding", b * EMBED_BATCH, total, "");
        let inputs: Vec<String> = batch
            .iter()
            .map(|(_, t)| format!("search_document: {}", t.chars().take(6000).collect::<String>()))
            .collect();
        let vecs = llm::embed(cfg, &inputs).await?;
        let conn = db.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        for ((id, _), v) in batch.iter().zip(vecs) {
            tx.execute("UPDATE chunks SET embedding = ?1 WHERE id = ?2", params![rag::to_blob(&v), id])?;
        }
        tx.commit()?;
        report.chunks_embedded += batch.len();
    }

    db.lock()
        .unwrap()
        .execute("UPDATE folders SET last_ingested = ?1 WHERE id = ?2", params![crate::db::now(), folder_id])?;
    emit("done", total, total, "");
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_secrets_and_binaries() {
        assert!(classify(Path::new("proj/.env")).is_none());
        assert!(classify(Path::new("proj/.env.example")).is_none());
        assert!(classify(Path::new("proj/.flaskenv")).is_none());
        assert!(classify(Path::new("proj/app.db")).is_none());
        assert!(classify(Path::new("proj/photo.jpg")).is_none());
        assert_eq!(classify(Path::new("proj/app.py")), Some(Kind::Code));
        assert_eq!(classify(Path::new("Notes.PDF")), Some(Kind::Pdf));
    }

    #[test]
    fn redacts_hardcoded_keys() {
        let src = "SECRET_KEY = 'supersecretvalue123'\nAPI_KEY: \"abcdefghij\"\nname = 'romel'";
        let out = redact(src);
        assert!(!out.contains("supersecretvalue123"));
        assert!(!out.contains("abcdefghij"));
        assert!(out.contains("name = 'romel'"));
    }

    #[test]
    fn prose_chunks_are_bounded() {
        let para = "word ".repeat(100);
        let text = (0..30).map(|_| para.clone()).collect::<Vec<_>>().join("\n\n");
        let chunks = chunk_prose(&text);
        assert!(chunks.len() > 5);
        assert!(chunks.iter().all(|c| c.len() <= CHUNK_CHARS * 2));
    }

    #[test]
    fn code_chunks_overlap() {
        let text = (0..130).map(|i| format!("line_{i} = {i}")).collect::<Vec<_>>().join("\n");
        let chunks = chunk_code(&text);
        assert_eq!(chunks.len(), 3);
        assert!(chunks[1].starts_with("line_55"));
    }
}
