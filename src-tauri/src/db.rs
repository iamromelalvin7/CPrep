//! SQLite storage: schema, migrations and seed data.

use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

pub struct Track {
    pub id: &'static str,
    pub name: &'static str,
    pub blurb: &'static str,
    pub accent: &'static str,
    pub status: &'static str,
    /// Seed outline the curriculum engine starts from.
    pub outline: &'static str,
}

pub const TRACKS: &[Track] = &[
    Track {
        id: "backend",
        name: "Backend Engineering",
        blurb: "APIs, frameworks, auth, data and deployment.",
        accent: "#E8A33D",
        status: "active",
        outline: "Python fluency, HTTP & REST design, FastAPI, Flask, Django, auth (sessions, JWT, OAuth), SQL & Postgres (joins, indexes, transactions, migrations), ORMs, caching with Redis, background jobs & queues, testing, Docker, deployment, Spring Boot (Java), Node/Express (after JavaScript).",
    },
    Track {
        id: "ai-eng",
        name: "AI Engineering",
        blurb: "LLM apps, RAG, agents, evals and serving.",
        accent: "#8FB573",
        status: "active",
        outline: "LLM APIs, prompting, structured outputs, embeddings & vector search, RAG pipelines, tool use & agents, evals, fine-tuning concepts, serving, latency & cost.",
    },
    Track {
        id: "databases",
        name: "Databases",
        blurb: "SQL, Postgres, modelling and performance.",
        accent: "#5B9BD5",
        status: "active",
        outline: "Relational model, SQL basics, joins, aggregation, normalization, indexes, transactions & isolation, Postgres features, query plans, migrations, NoSQL trade-offs.",
    },
    Track {
        id: "dsa",
        name: "Data Structures & Algorithms",
        blurb: "Patterns that win interviews.",
        accent: "#D9B44A",
        status: "active",
        outline: "Complexity, arrays & strings, hashing, two pointers, sliding window, stacks & queues, linked lists, trees & BST, heaps, graphs (BFS, DFS, Dijkstra), dynamic programming, interview patterns.",
    },
    Track {
        id: "system-design",
        name: "System Design",
        blurb: "Scaling, trade-offs and architecture.",
        accent: "#B07CC6",
        status: "active",
        outline: "Scalability, load balancing, caching, database replication & sharding, CAP, queues & streams, rate limiting, consistency, design-X exercises.",
    },
    Track {
        id: "networking",
        name: "Networking",
        blurb: "From packets to protocols.",
        accent: "#4FB3A9",
        status: "active",
        outline: "OSI & TCP/IP, IP addressing & subnetting, TCP vs UDP, DNS, HTTP & TLS, routing basics, sockets in Python, troubleshooting tools.",
    },
    Track {
        id: "automation",
        name: "AI Automation",
        blurb: "Workflows that help businesses scale.",
        accent: "#E58FA0",
        status: "active",
        outline: "Workflow tools, webhooks, APIs & scraping, LLM-powered automations, agentic workflows, client discovery & scoping, portfolio projects.",
    },
    Track {
        id: "interview",
        name: "Interview Prep",
        blurb: "DSA drills, design mocks, behavioural.",
        accent: "#C9C2B2",
        status: "active",
        outline: "DSA drills, system design mocks, behavioural STAR stories, resume & portfolio, remote-role applications.",
    },
    Track {
        id: "hardware",
        name: "Embedded & Edge AI",
        blurb: "Paused for now.",
        accent: "#6E6A62",
        status: "paused",
        outline: "Embedded systems, edge AI, robotics.",
    },
];

pub fn track(id: &str) -> Option<&'static Track> {
    TRACKS.iter().find(|t| t.id == id)
}

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    init(&conn)?;
    Ok(conn)
}

pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS folders (
            id INTEGER PRIMARY KEY,
            track_id TEXT NOT NULL,
            path TEXT NOT NULL,
            last_ingested TEXT,
            UNIQUE(track_id, path)
        );
        CREATE TABLE IF NOT EXISTS documents (
            id INTEGER PRIMARY KEY,
            folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
            path TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            kind TEXT NOT NULL,
            mtime INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS chunks (
            id INTEGER PRIMARY KEY,
            doc_id INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
            ord INTEGER NOT NULL,
            page INTEGER,
            text TEXT NOT NULL,
            embedding BLOB
        );
        CREATE INDEX IF NOT EXISTS chunks_doc ON chunks(doc_id);

        CREATE TABLE IF NOT EXISTS placements (
            track_id TEXT PRIMARY KEY,
            known TEXT NOT NULL,
            quiz TEXT,
            result TEXT,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS curricula (
            id INTEGER PRIMARY KEY,
            track_id TEXT NOT NULL,
            body TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS lessons (
            id INTEGER PRIMARY KEY,
            curriculum_id INTEGER NOT NULL REFERENCES curricula(id) ON DELETE CASCADE,
            ord INTEGER NOT NULL,
            phase_idx INTEGER NOT NULL,
            module_idx INTEGER NOT NULL,
            phase TEXT NOT NULL,
            module TEXT NOT NULL,
            title TEXT NOT NULL,
            objectives TEXT NOT NULL,
            status TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY,
            lesson_id INTEGER NOT NULL REFERENCES lessons(id) ON DELETE CASCADE,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            llm_content TEXT NOT NULL,
            sources TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS usage (
            id INTEGER PRIMARY KEY,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            cache_read INTEGER NOT NULL,
            cache_write INTEGER NOT NULL,
            cost_usd REAL NOT NULL,
            created_at TEXT NOT NULL
        );
        "#,
    )?;
    Ok(())
}

pub fn now() -> String {
    chrono::Local::now().to_rfc3339()
}

pub fn record_usage(conn: &Connection, u: &crate::llm::Usage) -> Result<()> {
    conn.execute(
        "INSERT INTO usage (provider, model, input_tokens, output_tokens, cache_read, cache_write, cost_usd, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![u.provider, u.model, u.input, u.output, u.cache_read, u.cache_write, u.cost_usd(), now()],
    )?;
    Ok(())
}
