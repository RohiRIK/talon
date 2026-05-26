# Embedding-Based Retrieval (Optional Feature)

> **Status:** ✅ Complete
> **Category:** Memory System

---

## 1. Design Philosophy

Semantic search is an **opt-in feature flag**, not a default.

```toml
# config.toml
[memory.embeddings]
enabled = false              # default: off
model = "nomic-embed-text"   # fastembed-rs model
dim = 768
backend = "sqlite_vec"       # or "qdrant"
```

Why optional:
- FTS5 covers 90% of use cases with zero overhead
- Embedding models require 200–500MB download
- fastembed-rs pulls in `ort` (ONNX Runtime) — heavy compile dep
- Semantic search shines for prose, not code/commands

Enable it when: corpus is large prose (docs, emails, articles) and
keyword search feels like it's missing conceptual matches.

---

## 2. Cargo Feature Flag

```toml
# Cargo.toml
[features]
default = []
embeddings = ["dep:fastembed", "dep:sqlite-vec"]

[dependencies]
fastembed = { version = "3", optional = true }
sqlite-vec = { version = "0.1", optional = true }
```

```rust
// Compile-time gate
#[cfg(feature = "embeddings")]
pub mod embeddings;

#[cfg(feature = "embeddings")]
pub use embeddings::EmbeddingStore;
```

---

## 3. fastembed-rs Integration

```rust
#[cfg(feature = "embeddings")]
pub struct EmbeddingEngine {
    model: fastembed::TextEmbedding,
    dim: usize,
}

#[cfg(feature = "embeddings")]
impl EmbeddingEngine {
    pub fn new(model_name: &str) -> Result<Self, EmbedError> {
        // Model downloaded to ~/.cache/fastembed/ on first use
        let model = fastembed::TextEmbedding::try_new(
            fastembed::InitOptions::new(
                fastembed::EmbeddingModel::from_str(model_name)?
            )
            .with_show_download_progress(true)
        )?;

        let dim = model.get_embedding_dim();

        tracing::info!(model = model_name, dim, "Embedding model loaded");

        Ok(Self { model, dim })
    }

    /// Embed a single text (blocking — call from spawn_blocking)
    pub fn embed_one(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let embeddings = self.model.embed(vec![text], None)?;
        Ok(embeddings.into_iter().next().unwrap_or_default())
    }

    /// Batch embed (more efficient than one-at-a-time)
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(self.model.embed(texts.to_vec(), None)?)
    }
}
```

---

## 4. sqlite-vec Storage

`sqlite-vec` is a SQLite extension for storing and querying float32 vectors.
No separate vector DB process needed — everything stays in the same SQLite file.

```sql
-- Requires sqlite-vec extension loaded
CREATE VIRTUAL TABLE message_vecs USING vec0(
    message_id INTEGER PRIMARY KEY,
    embedding FLOAT[768]  -- dim matches model
);
```

```rust
#[cfg(feature = "embeddings")]
impl MemoryStore {
    pub fn load_vec_extension(&self) -> Result<(), MemoryError> {
        self.db.query(|conn| {
            // Load sqlite-vec shared library
            unsafe {
                conn.load_extension(
                    std::path::Path::new("sqlite_vec"),
                    None,
                )?;
            }
            Ok(())
        }).await?
    }

    pub async fn upsert_embedding(
        &self,
        message_id: i64,
        embedding: Vec<f32>,
    ) -> Result<(), MemoryError> {
        // Serialize f32 vec to bytes (sqlite-vec format)
        let bytes: Vec<u8> = embedding.iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();

        self.db.query(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO message_vecs(message_id, embedding)
                 VALUES (?1, ?2)",
                params![message_id, bytes],
            )
        }).await??;

        Ok(())
    }

    pub async fn search_similar(
        &self,
        query_embedding: Vec<f32>,
        limit: usize,
        distance_threshold: f32,
    ) -> Result<Vec<SemanticHit>, MemoryError> {
        let bytes: Vec<u8> = query_embedding.iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();

        self.db.query(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT
                    v.message_id,
                    m.session_id,
                    m.role,
                    m.content,
                    vec_distance_cosine(v.embedding, ?1) AS distance
                 FROM message_vecs v
                 JOIN messages m ON v.message_id = m.id
                 WHERE distance < ?2
                 ORDER BY distance ASC
                 LIMIT ?3"
            )?;

            stmt.query_map(
                params![bytes, distance_threshold, limit as i64],
                |row| Ok(SemanticHit {
                    message_id: row.get(0)?,
                    session_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap(),
                    role: row.get(2)?,
                    content: row.get(3)?,
                    distance: row.get(4)?,
                }),
            )?.collect::<rusqlite::Result<_>>()
        }).await?
    }
}
```

---

## 5. Hybrid Search (FTS5 + Embeddings)

When embeddings are enabled, `session_search` uses Reciprocal Rank Fusion:

```rust
pub async fn hybrid_search(
    &self,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, MemoryError> {
    // Parallel: BM25 keyword + cosine semantic
    let (fts_hits, sem_hits) = tokio::join!(
        self.search_messages(query, limit * 3),
        self.semantic_search(query, limit * 3),
    );

    let fts_hits = fts_hits?;
    let sem_hits = sem_hits.unwrap_or_default();  // graceful if embeddings fail

    // Reciprocal Rank Fusion
    let k = 60.0_f32;
    let mut scores: HashMap<i64, f32> = HashMap::new();

    for (rank, hit) in fts_hits.iter().enumerate() {
        *scores.entry(hit.message_id).or_default() += 1.0 / (k + rank as f32 + 1.0);
    }
    for (rank, hit) in sem_hits.iter().enumerate() {
        *scores.entry(hit.message_id).or_default() += 1.0 / (k + rank as f32 + 1.0);
    }

    let mut ranked: Vec<(i64, f32)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    // Fetch full message data for top results
    let top_ids: Vec<i64> = ranked.into_iter().take(limit).map(|(id, _)| id).collect();
    self.fetch_messages_by_ids(&top_ids).await
}
```

---

## 6. Background Embedding Pipeline

New messages get embedded asynchronously — never blocking the agent loop:

```rust
pub struct EmbeddingPipeline {
    queue_tx: mpsc::Sender<EmbedJob>,
}

impl EmbeddingPipeline {
    pub fn start(
        engine: Arc<EmbeddingEngine>,
        store: Arc<MemoryStore>,
    ) -> Self {
        let (tx, mut rx) = mpsc::channel::<EmbedJob>(256);

        tokio::spawn(async move {
            let mut batch = vec![];
            let mut interval = tokio::time::interval(Duration::from_millis(200));

            loop {
                tokio::select! {
                    job = rx.recv() => {
                        match job {
                            Some(j) => {
                                batch.push(j);
                                if batch.len() >= 32 { flush(&engine, &store, &mut batch).await; }
                            }
                            None => {
                                flush(&engine, &store, &mut batch).await;
                                break;
                            }
                        }
                    }
                    _ = interval.tick() => {
                        if !batch.is_empty() {
                            flush(&engine, &store, &mut batch).await;
                        }
                    }
                }
            }
        });

        Self { queue_tx: tx }
    }

    pub fn enqueue(&self, message_id: i64, text: String) {
        let _ = self.queue_tx.try_send(EmbedJob { message_id, text });
        // try_send: drop if queue full — embedding is best-effort
    }
}

async fn flush(engine: &EmbeddingEngine, store: &MemoryStore, batch: &mut Vec<EmbedJob>) {
    if batch.is_empty() { return; }
    let texts: Vec<&str> = batch.iter().map(|j| j.text.as_str()).collect();
    let engine = engine.clone();

    match tokio::task::spawn_blocking(move || engine.embed_batch(&texts)).await {
        Ok(Ok(embeddings)) => {
            for (job, emb) in batch.iter().zip(embeddings) {
                store.upsert_embedding(job.message_id, emb).await.ok();
            }
        }
        Err(e) | Ok(Err(e)) => {
            tracing::warn!("Embedding batch failed: {e}");
        }
    }

    batch.clear();
}
```

---

## 7. Qdrant Alternative Backend

For large deployments (millions of messages), swap sqlite-vec for Qdrant:

```toml
[memory.embeddings]
enabled = true
backend = "qdrant"

[memory.embeddings.qdrant]
url = "http://localhost:6333"
collection = "talon_messages"
```

The `EmbeddingStore` trait abstracts both backends:

```rust
#[async_trait]
pub trait EmbeddingStore: Send + Sync {
    async fn upsert(&self, id: i64, embedding: Vec<f32>) -> Result<(), EmbedError>;
    async fn search(&self, query: Vec<f32>, limit: usize) -> Result<Vec<(i64, f32)>, EmbedError>;
    async fn delete(&self, id: i64) -> Result<(), EmbedError>;
}
```
---

## Related Documents

### Depends On
- [SQLite & FTS5 in Rust](55_SQLite_FTS5_In_Rust.md)
- [Context & Memory Architecture](../02_Architecture/15_Context_And_Memory_Architecture.md)

### See Also
- [FTS5 Search Deep Dive](58_FTS5_Search_Deep_Dive.md)
- [Memory System](../04_Core_Features/35_Memory_System_SQLite_FTS5.md)
- [Embedding-Based Retrieval (alt)](59a_Embedding_Based_Retrieval.md)

