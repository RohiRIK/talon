# 73 — LanceDB: Embedded Vector + FTS Database

> **Repo:** [lancedb/lancedb](https://github.com/lancedb/lancedb)  
> **Language:** Rust core, Rust/Python/JS SDKs  
> **Architecture:** Embedded (in-process, like SQLite)  
> **License:** Apache 2.0  
> **Maturity:** ~5k stars, v0.29, well-funded (LanceDB Inc)

---

## What It Is

A serverless, embedded vector database built on the Lance columnar format (Apache Arrow-based). Runs in-process — no server, no Docker, no network. Think "SQLite for vectors."

---

## Why It Matters for Talon

LanceDB could **replace SQLite + FTS5** as Talon's storage layer while **adding native vector search** — unifying structured data, full-text search, and semantic search in one embedded dependency.

---

## Architecture

```
Talon Process
    │
    ├── lancedb crate (in-process)
    │       │
    │       ├── Vector Index (IVF-PQ, HNSW)
    │       ├── Full-Text Search (Tantivy-based)
    │       ├── SQL-like queries
    │       └── Lance columnar storage
    │               │
    │               └── Local filesystem / S3 / GCS
    │
    └── No external process needed
```

### Key Properties
- **Embedded** — `cargo add lancedb`, no server
- **Vector + FTS** — both in one DB, hybrid search supported
- **Versioned** — automatic data versioning (free memory snapshots/rollback)
- **Arrow-native** — zero-copy interop with Arrow ecosystem
- **GPU-accelerated** — index building can use GPU
- **Cloud-ready** — same API for local files and S3/GCS

---

## Rust API

```rust
use lancedb::connect;

// Connect (creates local DB)
let db = connect("./talon-brain").execute().await?;

// Create table with schema
let table = db.create_table("memories", data).execute().await?;

// Vector search
let results = table
    .vector_search(embedding)
    .limit(10)
    .execute()
    .await?;

// Full-text search
let results = table
    .query()
    .full_text_search("auth patterns", "content")
    .limit(10)
    .execute()
    .await?;

// Hybrid (vector + FTS)
let results = table
    .query()
    .nearest_to(embedding)
    .full_text_search("auth", "content")
    .execute()
    .await?;
```

---

## Comparison: LanceDB vs SQLite + FTS5

| Feature | SQLite + FTS5 | LanceDB |
|---------|--------------|---------|
| Embedded | ✓ | ✓ |
| Structured queries | ✓ (SQL) | ✓ (SQL-like) |
| Full-text search | ✓ (FTS5) | ✓ (Tantivy) |
| Vector search | ✗ (needs extension) | ✓ (native) |
| Hybrid search | ✗ | ✓ |
| Versioning | ✗ | ✓ (free) |
| Rust ecosystem | rusqlite (mature) | lancedb (pre-1.0) |
| Binary size impact | Minimal | Larger (Arrow deps) |
| Battle-tested | Decades | Years |

**Trade-off:** LanceDB gives vector search + versioning but is pre-1.0 and adds heavier dependencies. SQLite is rock-solid but needs a separate vector solution.

---

## Integration Path for Talon

### Option A: LanceDB as Primary (replaces SQLite)
```
talon-memory/
├── backend.rs          # MemoryBackend trait
├── lance_store.rs      # LanceDB implementation
│   ├── memories table  # content + embeddings + metadata
│   ├── relations table # memory graph edges
│   └── context table   # project context items
├── working.rs          # conversation buffer (in-memory)
└── promotion.rs        # working → long-term promotion
```

### Option B: LanceDB alongside SQLite (hybrid)
- SQLite for structured data (config, projects, sessions)
- LanceDB for memory storage (vectors + FTS + metadata)
- Best of both worlds, two dependencies

### Option C: Feature-flagged
```toml
[features]
default = ["sqlite-memory"]    # SQLite + FTS5 (minimal deps)
lance-memory = ["lancedb"]     # LanceDB (vector + FTS unified)
```

---

## Risks

1. **Pre-1.0** — API may change, though the company is well-funded
2. **Binary size** — Arrow dependencies are not small
3. **Complexity** — more powerful than needed for simple key-value memories
4. **Maturity** — SQLite has decades of battle-testing; Lance has years

---

## Verdict

★★★★☆ — **Strongest storage candidate.** The embedded + vector + FTS combination is exactly what Talon needs. The pre-1.0 status and dependency weight are the only concerns. Recommend **Option C** (feature-flagged) so users can choose SQLite for minimal builds or LanceDB for full semantic search.
