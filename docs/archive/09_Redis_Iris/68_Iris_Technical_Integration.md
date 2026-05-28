# Redis Iris — Technical Integration with Talon

> **Status:** ✅ Done
> **Category:** 09_Redis_Iris

---

## SDK Landscape

| Language | Package | Maturity |
|----------|---------|----------|
| Python | `agent-memory-client` (PyPI) | Primary, full-featured |
| JavaScript | `agent-memory-client-js` | v0.3.1 |
| Java | `agent-memory-client-java` | Early |
| **Rust** | **None** | **Does not exist** |
| Any | REST API (FastAPI) | Language-agnostic |
| Any | MCP Server | Tool-based access |

**No Rust SDK exists.** The Agent Memory Server is a Python FastAPI application. Integration from Rust means either HTTP calls or native reimplementation.

---

## Integration Strategies

### Strategy 1: Sidecar + REST API

Run the Agent Memory Server as a Docker sidecar. Talon calls it via HTTP.

```
┌──────────────┐     HTTP      ┌───────────────────┐
│  Talon       │◄────────────►│  Agent Memory      │
│  (Rust)      │  localhost    │  Server (Python)   │
│              │  :6677       │                     │
└──────────────┘               └────────┬──────────┘
                                        │
                                   ┌────▼────┐
                                   │  Redis  │
                                   │  Stack  │
                                   └─────────┘
```

```rust
// talon-memory/src/redis_iris.rs
use reqwest::Client;

pub struct IrisMemoryClient {
    client: Client,
    base_url: String, // http://localhost:6677
}

impl IrisMemoryClient {
    pub async fn store_memory(&self, session_id: &str, content: &str) -> Result<()> {
        self.client.post(format!("{}/memory", self.base_url))
            .json(&serde_json::json!({
                "session_id": session_id,
                "content": content
            }))
            .send().await?;
        Ok(())
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<Memory>> {
        let resp = self.client.post(format!("{}/memory/search", self.base_url))
            .json(&serde_json::json!({
                "query": query,
                "limit": limit
            }))
            .send().await?
            .json::<Vec<Memory>>().await?;
        Ok(resp)
    }
}
```

**Pros:** Zero Rust-side complexity, uses battle-tested Python server.
**Cons:** Breaks single-binary story, adds Python + Docker dependency, network latency.

### Strategy 2: Native Rust Against Redis (Recommended)

Use the `redis` crate directly with RediSearch commands. No Python. No sidecar.

```rust
// talon-memory/src/redis_store.rs
use redis::{AsyncCommands, Client};

pub struct RedisMemoryStore {
    client: Client,
    embedder: Arc<dyn Embedder>,
}

impl RedisMemoryStore {
    pub async fn create_index(&self) -> Result<()> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        // Create RediSearch index with vector field
        redis::cmd("FT.CREATE")
            .arg("idx:memories")
            .arg("ON").arg("JSON")
            .arg("PREFIX").arg("1").arg("memory:")
            .arg("SCHEMA")
            .arg("$.content").arg("AS").arg("content").arg("TEXT")
            .arg("$.embedding").arg("AS").arg("embedding").arg("VECTOR")
            .arg("FLAT").arg("6")
            .arg("TYPE").arg("FLOAT32")
            .arg("DIM").arg("384")  // all-MiniLM-L6-v2
            .arg("DISTANCE_METRIC").arg("COSINE")
            .arg("$.session_id").arg("AS").arg("session_id").arg("TAG")
            .arg("$.user_id").arg("AS").arg("user_id").arg("TAG")
            .query_async(&mut conn).await?;
        Ok(())
    }

    pub async fn search_hybrid(&self, query: &str, limit: usize) -> Result<Vec<Memory>> {
        let embedding = self.embedder.embed(query).await?;
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        // Hybrid: vector KNN + full-text, fused with RRF
        let results: Vec<(String, f64, String)> = redis::cmd("FT.SEARCH")
            .arg("idx:memories")
            .arg(format!("({})=>[KNN {} @embedding $vec AS score]", query, limit))
            .arg("PARAMS").arg("2").arg("vec").arg(embedding.as_bytes())
            .arg("SORTBY").arg("score")
            .arg("LIMIT").arg("0").arg(limit)
            .query_async(&mut conn).await?;
        // Parse and return
        todo!()
    }
}
```

**Pros:** Pure Rust, no sidecar, sub-ms reads, leverages Redis ecosystem.
**Cons:** Requires Redis running, more implementation work.

### Strategy 3: MCP Bridge

Connect Talon's MCP client (Phase 5) to the Agent Memory MCP server.

**Pros:** Minimal code, uses existing MCP infrastructure.
**Cons:** Indirect, slower, limited to MCP tool semantics.

---

## Rust Crate Dependencies

```toml
# Cargo.toml additions for redis-memory feature
[dependencies]
redis = { version = "0.27", features = ["tokio-comp", "json"], optional = true }

[features]
redis-memory = ["redis"]
```

The `redis` crate supports:
- Async via Tokio (`tokio-comp` feature)
- RediSearch commands (raw `redis::cmd()`)
- RedisJSON (`json` feature)
- Connection pooling (`bb8-redis` or `deadpool-redis`)
- TLS (`tls-rustls` feature)

---

## Redis Data Model for Talon

```
Redis Key Structure:
  session:{session_id}:messages    → List of message JSONs
  session:{session_id}:summary     → Latest session summary
  memory:{uuid}                    → JSON: {content, embedding, user_id, session_id, topic, entities, created_at}
  cache:llm:{hash}                 → Cached LLM response (TTL-based)
  user:{user_id}:facts             → Set of memory UUIDs for this user

Redis Search Indexes:
  idx:memories  → Vector + full-text over memory:* keys
  idx:sessions  → Full-text over session summaries
```

---

## Performance Characteristics

| Operation | SQLite+FTS5 | Redis Search | Winner |
|-----------|-------------|-------------|--------|
| Keyword search (10K docs) | ~5ms | ~1ms | Redis |
| Vector search (10K vectors) | ~50ms (fastembed) | ~2ms | Redis |
| Write throughput | ~10K/s (WAL) | ~100K/s | Redis |
| Durability | Excellent (file) | Needs AOF/RDB | SQLite |
| Memory usage | Disk-based | All in RAM | SQLite |
| Zero dependencies | ✅ | ❌ (needs Redis) | SQLite |

**Recommendation:** SQLite for durability + offline. Redis for speed + scale. Hybrid for best of both.

---

## Feature Flag Design

```rust
// talon-memory/src/lib.rs
pub enum MemoryBackend {
    Sqlite(SqliteStore),       // Always available
    #[cfg(feature = "redis-memory")]
    Redis(RedisMemoryStore),   // Optional
    #[cfg(feature = "redis-memory")]
    Hybrid {                   // Redis hot + SQLite durable
        hot: RedisMemoryStore,
        durable: SqliteStore,
    },
}

impl MemoryStore for MemoryBackend {
    // Dispatch to active backend
}
```

Config in `~/.talon/config.toml`:
```toml
[memory]
backend = "sqlite"  # or "redis" or "hybrid"

[memory.redis]
url = "redis://localhost:6379"
```

---

## Related Documents

### Depends On
- [Redis Iris Overview](66_Redis_Iris_Overview.md)
- [Redis Iris Two-Tier Memory](67_Iris_Two_Tier_Memory.md)

### Used By
- [Memory System (SQLite + FTS5)](../04_Core_Features/35_Memory_System_SQLite_FTS5.md)
- [Embedding-based Semantic Retrieval](../07_Memory_System/59_Embedding_Based_Retrieval.md)

### See Also
- [Redis Iris Philosophy](69_Iris_Philosophy.md)
- [Redis Iris Semantic Cache](70_Iris_Semantic_Cache.md)
