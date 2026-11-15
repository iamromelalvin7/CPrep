//! Retrieval: brute-force cosine similarity over a track's chunk embeddings.
//! A few thousand chunks per track search in milliseconds, so no vector extension is needed yet.

use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::config::Config;
use crate::llm;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub chunk_id: i64,
    pub title: String,
    pub path: String,
    pub page: Option<i64>,
    pub text: String,
    pub score: f32,
}

pub fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

pub fn from_blob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na.sqrt() * nb.sqrt()) }
}

/// Rank stored chunks against a query vector; at most `per_doc` chunks from one file, for variety.
pub fn rank(conn: &Connection, track_id: &str, qv: &[f32], k: usize, per_doc: usize) -> Result<Vec<Source>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, d.title, d.path, c.page, c.text, c.embedding, d.id
         FROM chunks c JOIN documents d ON d.id = c.doc_id JOIN folders f ON f.id = d.folder_id
         WHERE f.track_id = ?1 AND c.embedding IS NOT NULL",
    )?;
    let mut scored: Vec<(Source, i64)> = stmt
        .query_map([track_id], |r| {
            let blob: Vec<u8> = r.get(5)?;
            Ok((
                Source {
                    chunk_id: r.get(0)?,
                    title: r.get(1)?,
                    path: r.get(2)?,
                    page: r.get(3)?,
                    text: r.get(4)?,
                    score: cosine(qv, &from_blob(&blob)),
                },
                r.get::<_, i64>(6)?,
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    scored.sort_by(|a, b| b.0.score.total_cmp(&a.0.score));
    let mut per: HashMap<i64, usize> = HashMap::new();
    Ok(scored
        .into_iter()
        .filter(|(_, doc)| {
            let n = per.entry(*doc).or_default();
            *n += 1;
            *n <= per_doc
        })
        .map(|(s, _)| s)
        .take(k)
        .collect())
}

pub async fn search(cfg: &Config, db: &Mutex<Connection>, track_id: &str, query: &str, k: usize) -> Result<Vec<Source>> {
    let has_chunks: bool = db.lock().unwrap().query_row(
        "SELECT EXISTS(SELECT 1 FROM chunks c JOIN documents d ON d.id = c.doc_id
         JOIN folders f ON f.id = d.folder_id WHERE f.track_id = ?1 AND c.embedding IS NOT NULL)",
        [track_id],
        |r| r.get(0),
    )?;
    if !has_chunks {
        return Ok(vec![]);
    }
    let qv = llm::embed(cfg, &[format!("search_query: {query}")]).await?.remove(0);
    let conn = db.lock().unwrap();
    rank(&conn, track_id, &qv, k, 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    #[test]
    fn blob_roundtrip_and_cosine() {
        let v = vec![0.5f32, -1.25, 3.0];
        assert_eq!(from_blob(&to_blob(&v)), v);
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
    }

    #[test]
    fn ranks_by_similarity_within_track() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init(&conn).unwrap();
        conn.execute("INSERT INTO folders (id, track_id, path) VALUES (1, 'backend', 'x'), (2, 'dsa', 'y')", []).unwrap();
        conn.execute(
            "INSERT INTO documents (id, folder_id, path, title, kind, mtime) VALUES
             (1, 1, 'a', 'flask.md', 'text', 0), (2, 1, 'b', 'sql.md', 'text', 0), (3, 2, 'c', 'heap.md', 'text', 0)",
            [],
        )
        .unwrap();
        for (id, doc, v) in [(1, 1, [1.0f32, 0.0]), (2, 2, [0.0, 1.0]), (3, 3, [1.0, 0.0])] {
            conn.execute(
                "INSERT INTO chunks (id, doc_id, ord, text, embedding) VALUES (?1, ?2, 0, 't', ?3)",
                params![id, doc, to_blob(&v)],
            )
            .unwrap();
        }
        let hits = rank(&conn, "backend", &[0.9, 0.1], 5, 2).unwrap();
        assert_eq!(hits.len(), 2, "other tracks' chunks must not leak in");
        assert_eq!(hits[0].title, "flask.md");
    }
}
