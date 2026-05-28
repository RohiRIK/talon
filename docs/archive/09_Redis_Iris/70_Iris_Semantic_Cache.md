# Redis Iris — Semantic Cache & LLM Cost Optimization

> **Status:** ✅ Done
> **Category:** 09_Redis_Iris

---

## The Problem: LLM Calls Are Expensive

Every LLM call costs time (1-10s latency) and money ($0.001-$0.10+ per call). Agents make many calls per session — tool dispatch, fact extraction, summarization. Cron jobs repeat similar prompts hourly. Without caching, costs compound linearly.

---

## LangCache: Semantic Response Caching

Iris's LangCache doesn't do exact string matching. It embeds the prompt, searches for semantically similar cached prompts, and returns the cached response if similarity exceeds a threshold.

```
User prompt: "What's the weather in New York?"
    │
    ▼
Embed prompt → vector
    │
    ▼
Search cache for similar vectors (cosine > 0.95)
    │
    ├── Cache HIT: "NYC weather today?" (similarity: 0.97)
    │       ▼
    │   Return cached response (0ms, $0)
    │
    └── Cache MISS
            ▼
        Call LLM (2s, $0.01)
            │
            ▼
        Store response in cache with TTL
```

---

## Talon Implementation

```rust
// talon-memory/src/cache.rs
pub struct SemanticCache {
    store: Arc<dyn CacheStore>,    // Redis or in-memory
    embedder: Arc<dyn Embedder>,
    similarity_threshold: f32,      // 0.95 default
    ttl: Duration,                  // 1 hour default
}

impl SemanticCache {
    pub async fn get_or_compute<F, Fut>(
        &self,
        prompt: &str,
        system: &str,
        compute: F,
    ) -> Result<LlmResponse>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<LlmResponse>>,
    {
        let key = format!("{}:{}", system, prompt);
        let embedding = self.embedder.embed(&key).await?;

        // Check cache
        if let Some(cached) = self.store
            .find_similar(&embedding, self.similarity_threshold)
            .await?
        {
            tracing::debug!(similarity = cached.similarity, "semantic cache hit");
            return Ok(cached.response);
        }

        // Cache miss — compute and store
        let response = compute().await?;
        self.store.insert(&key, &embedding, &response, self.ttl).await?;
        Ok(response)
    }
}
```

### Cache Backends

```rust
pub enum CacheBackend {
    /// In-memory LRU — works in single-binary mode
    InMemory(LruCache<Vec<f32>, CachedResponse>),
    /// Redis — persistent, shared across restarts
    #[cfg(feature = "redis-memory")]
    Redis(RedisCacheStore),
}
```

### What to Cache vs. Not

| Cache? | Scenario | Reason |
|--------|----------|--------|
| ✅ | Cron job repeated prompts | Same question, same answer |
| ✅ | Tool schema descriptions | Stable across sessions |
| ✅ | Fact extraction prompts | Similar conversations → similar facts |
| ❌ | User conversations | Context changes every turn |
| ❌ | Tool execution results | Side effects, real-time data |
| ❌ | Approval decisions | Must be fresh every time |

### Cache Invalidation

- **TTL-based:** Default 1 hour. Configurable per cache entry type.
- **Manual flush:** `talon cache clear` CLI command.
- **Capacity-based:** LRU eviction when cache exceeds size limit.

---

## Cost Impact Estimate

For a typical Talon deployment with 3 cron jobs running hourly:

| Without Cache | With Cache |
|--------------|------------|
| 72 LLM calls/day | ~20 LLM calls/day (72% cache hit rate) |
| ~$0.72/day | ~$0.20/day |
| ~$21.60/month | ~$6.00/month |

The savings scale with repetition. More cron jobs = higher cache hit rate.

---

## Config

```toml
# ~/.talon/config.toml
[cache]
enabled = true
backend = "memory"  # or "redis"
similarity_threshold = 0.95
ttl_seconds = 3600
max_entries = 10000

[cache.redis]
url = "redis://localhost:6379/1"  # separate DB from memory
```

---

## Related Documents

### Depends On
- [Redis Iris Overview](66_Redis_Iris_Overview.md)
- [Redis Iris Technical Integration](68_Iris_Technical_Integration.md)

### See Also
- [Redis Iris Philosophy](69_Iris_Philosophy.md)
- [LLM Provider Abstraction](../05_API_Bindings/41_LLM_Provider_Abstraction.md)
