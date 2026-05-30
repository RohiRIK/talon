# Embedding-Based Retrieval

> **Status:** Spec aligned with the memory decision — [ADR 0008: SQLite + sqlite-vec](../ADR/0008-sqlite-vec-memory-backend.md) (supersedes the LanceDB ADR 0005). This file already specced `sqlite-vec`; ADR 0008 makes the project-wide decision match it.
> **Category:** Memory System

---

## 1. Default vs. Semantic Search

Talon defaults to [FTS5 (SQLite](55_SQLite_FTS5_In_Rust.md) full-text search) for session and memory
retrieval. FTS5 is:
- Zero setup (no model download)
- Blazing fast (C extension to SQLite)
- Good enough for keyword-based recall

Semantic (embedding) search is opt-in and adds:
- Concept-level matching ("authentication issues" finds "JWT token expired" without keyword overlap)
- Multilingual recall
- Fuzzy paraphrase matching

---

## 2. fastembed-rs Integration

`[fastembed](59_Embedding_Retrieval.md)-rs` is the Rust port of FastEmbed — generates embeddings locally
without a server, using ONNX models.

```toml
# talon-memory/Cargo.toml
[dependencies]
fastembed = { version = "3", optional = true }

[features]
semantic-search = ["fastembed"]
```

```rust
#[cfg(feature = "semantic-search")]
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

#[cfg(feature = "semantic-search")]
pub struct EmbeddingEngine {
    model: TextEmbedding,
}

#[cfg(feature = "semantic-search")]
impl EmbeddingEngine {
    pub fn new() -> Result<Self, EmbeddingError> {
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::AllMiniLML6V2)
                .with_show_download_progress(true)
        )?;
        Ok(Self { model })
    }

    pub fn embed(&self, texts: Vec<&str>) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        self.model.embed(texts, None)
            .map_err(EmbeddingError::Fastembed)
    }
}
```

The default model `AllMiniLML6V2` produces 384-dimensional vectors and is
~80MB. It runs entirely on CPU with no GPU requirement.

---

## 3. Vector Storage (sqlite-vec)

Rather than a separate vector database, Talon uses `sqlite-vec` — a SQLite
extension that adds vector similarity search to the **same** `talon.db`. A fact,
its FTS5 row, and its embedding commit in one transaction (no dual-write).

**Single-binary requirement:** the extension is **statically compiled in** (the
`sqlite-vec` crate bundles the C source) and registered as an *auto-extension*
**before** any pooled connection is opened — never `load_extension` from an
external `.so`, which would break the single-binary guarantee. Register once at
`Database::open`:

```rust
// Once, before the deadpool pool opens any connection. Applies to all
// connections opened afterwards.
unsafe {
    rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
        sqlite_vec::sqlite3_vec_init as *const (),
    )));
}
```

Then the schema (canonical names in PLAN.md task 2.5.2 — `memories`,
`memories_fts`, `vec_memories`):

```rust
conn.execute_batch(r#"
    CREATE VIRTUAL TABLE IF NOT EXISTS vec_memories USING vec0(
        embedding float[384]
    );
    -- `memories` / `memories_fts` are created by the SQLite migration set.
"#)?;
```

> The example below uses illustrative table names; the canonical schema
> (`memories`, `memories_fts`, `vec_memories`) lives in PLAN.md task 2.5.2.

```rust
pub fn upsert_embedding(
    conn: &Connection,
    content: &str,
    source: &str,
    embedding: &[f32],
) -> Result<i64, rusqlite::Error> {
    // Insert content
    conn.execute(
        "INSERT INTO memory_entries (content, source, created_at)
         VALUES (?1, ?2, unixepoch())",
        params![content, source],
    )?;
    let id = conn.last_insert_rowid();

    // Insert vector
    conn.execute(
        "INSERT INTO memory_vecs (rowid, embedding) VALUES (?1, ?2)",
        params![id, embedding.as_bytes()],
    )?;

    Ok(id)
}

pub fn search_similar(
    conn: &Connection,
    query_embedding: &[f32],
    limit: u32,
) -> Result<Vec<MemorySearchResult>, rusqlite::Error> {
    let mut stmt = conn.prepare(r#"
        SELECT
            me.id,
            me.content,
            me.source,
            distance
        FROM memory_vecs
        JOIN memory_entries me ON me.id = memory_vecs.rowid
        WHERE embedding MATCH ?1
          AND k = ?2
        ORDER BY distance
    "#)?;

    stmt.query_map(params![query_embedding.as_bytes(), limit], |row| {
        Ok(MemorySearchResult {
            id: row.get(0)?,
            content: row.get(1)?,
            source: row.get(2)?,
            distance: row.get(3)?,
        })
    })?.collect()
}
```

---

## 4. Hybrid Retrieval

For best results, Talon uses reciprocal rank fusion (RRF) to combine
FTS5 keyword scores with vector similarity scores:

```rust
pub async fn hybrid_search(
    db: &MemoryDb,
    engine: Option<&EmbeddingEngine>,
    query: &str,
    limit: u32,
) -> Result<Vec<MemorySearchResult>> {
    // Always: FTS5 results
    let fts_results = db.fts_search(query, limit * 2).await?;

    // Optional: vector results
    let vec_results = if let Some(eng) = engine {
        let embedding = eng.embed(vec![query])?;
        db.vector_search(&embedding[0], limit * 2).await?
    } else {
        vec![]
    };

    // RRF fusion: score = 1 / (rank + 60)
    let mut scores: HashMap<i64, f64> = HashMap::new();

    for (rank, result) in fts_results.iter().enumerate() {
        *scores.entry(result.id).or_default() += 1.0 / (rank as f64 + 60.0);
    }
    for (rank, result) in vec_results.iter().enumerate() {
        *scores.entry(result.id).or_default() += 1.0 / (rank as f64 + 60.0);
    }

    // Re-rank by RRF score
    let mut all: Vec<_> = scores.into_iter().collect();
    all.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    all.truncate(limit as usize);

    // Fetch content for top results
    let ids: Vec<i64> = all.iter().map(|(id, _)| *id).collect();
    db.fetch_by_ids(&ids).await
}
```

---

## 5. When Embeddings Are Indexed

New content is indexed asynchronously after it's written:

```rust
// After a session message is saved:
if let Some(engine) = &self.embedding_engine {
    let embedding = engine.embed(vec![&content])?;
    db.upsert_embedding(&content, "session", &embedding[0]).await?;
}

// After MEMORY.md is updated, re-index changed entries:
memory_indexer.reindex_file(&memory_md_path).await?;
```

Indexing runs in a `spawn_blocking` call to avoid blocking the async runtime.

---

## 6. Configuration

```toml
# ~/.talon/profiles/default/config.toml
[memory]
fts_enabled = true
semantic_search = false     # opt-in
embedding_model = "AllMiniLML6V2"   # 80MB, 384-dim
vector_db = "sqlite-vec"    # or "qdrant" for external
```
---

## Related Documents

### Depends On
- [SQLite & FTS5 in Rust](55_SQLite_FTS5_In_Rust.md)

### See Also
- [Embedding Retrieval (main)](59_Embedding_Retrieval.md)
- [FTS5 Search Deep Dive](58_FTS5_Search_Deep_Dive.md)

