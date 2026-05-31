//! `SemanticCache` — embedding-keyed LLM response cache (task 2.5.9).
//!
//! Caches responses keyed by the *meaning* of a prompt rather than its exact
//! bytes: a lookup embeds the prompt and returns a cached response when an entry
//! exceeds the similarity threshold (default 0.95). Bounded LRU with a TTL, all
//! in-process — no Redis (Iris LangCache pattern, doc #70).
//!
//! Brute-force cosine over the entries is fine at cache scale (tens–hundreds).
//! Embeddings are caller-supplied (the gateway owns the fastembed model), so the
//! cache itself depends only on [`crate::cosine_similarity`].

use std::time::{Duration, Instant};

use crate::cosine_similarity;

/// Default cosine similarity at or above which a cached response is reused.
pub const SEMANTIC_CACHE_THRESHOLD: f32 = 0.95;

/// Snapshot of cache occupancy and hit/miss counters (for `talon cache stats`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    pub entries: usize,
    pub capacity: usize,
    pub hits: u64,
    pub misses: u64,
}

struct Entry {
    embedding: Vec<f32>,
    response: String,
    inserted_at: Instant,
    last_used: Instant,
}

/// In-memory semantic cache with LRU eviction and per-entry TTL.
pub struct SemanticCache {
    capacity: usize,
    ttl: Duration,
    threshold: f32,
    entries: Vec<Entry>,
    hits: u64,
    misses: u64,
}

impl SemanticCache {
    /// Cache holding at most `capacity` entries, each living for `ttl`.
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self::with_threshold(capacity, ttl, SEMANTIC_CACHE_THRESHOLD)
    }

    /// As [`Self::new`] but with a custom similarity threshold (clamped `[0,1]`).
    pub fn with_threshold(capacity: usize, ttl: Duration, threshold: f32) -> Self {
        Self {
            capacity,
            ttl,
            threshold: threshold.clamp(0.0, 1.0),
            entries: Vec::new(),
            hits: 0,
            misses: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            entries: self.entries.len(),
            capacity: self.capacity,
            hits: self.hits,
            misses: self.misses,
        }
    }

    /// Drop all entries (keeps the hit/miss counters). Backs `talon cache clear`.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Look up a response by prompt embedding. Returns a cached response when an
    /// entry meets the similarity threshold; refreshes that entry's LRU position.
    pub fn get(&mut self, embedding: &[f32]) -> Option<String> {
        self.get_at(embedding, Instant::now())
    }

    /// Insert a response keyed by its prompt embedding, evicting the LRU entry if
    /// over capacity.
    pub fn put(&mut self, embedding: &[f32], response: impl Into<String>) {
        self.put_at(embedding, response.into(), Instant::now());
    }

    fn prune_expired(&mut self, now: Instant) {
        let ttl = self.ttl;
        self.entries
            .retain(|e| now.saturating_duration_since(e.inserted_at) < ttl);
    }

    fn get_at(&mut self, embedding: &[f32], now: Instant) -> Option<String> {
        self.prune_expired(now);
        let mut best: Option<(usize, f32)> = None;
        for (i, e) in self.entries.iter().enumerate() {
            let sim = cosine_similarity(embedding, &e.embedding);
            if sim >= self.threshold && best.is_none_or(|(_, b)| sim > b) {
                best = Some((i, sim));
            }
        }
        match best {
            Some((i, _)) => {
                self.entries[i].last_used = now;
                self.hits += 1;
                Some(self.entries[i].response.clone())
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    fn put_at(&mut self, embedding: &[f32], response: String, now: Instant) {
        self.prune_expired(now);
        self.entries.push(Entry {
            embedding: embedding.to_vec(),
            response,
            inserted_at: now,
            last_used: now,
        });
        while self.entries.len() > self.capacity {
            let Some(lru) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(i, _)| i)
            else {
                break;
            };
            self.entries.remove(lru);
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn emb(a: f32, b: f32) -> Vec<f32> {
        let mut v = vec![0.0f32; 8];
        v[0] = a;
        v[1] = b;
        v
    }

    fn cache() -> SemanticCache {
        SemanticCache::new(3, Duration::from_secs(60))
    }

    #[test]
    fn miss_on_empty_cache() {
        let mut c = cache();
        assert!(c.get(&emb(1.0, 0.0)).is_none());
        assert_eq!(c.stats().misses, 1);
    }

    #[test]
    fn hit_on_identical_embedding() {
        let mut c = cache();
        c.put(&emb(1.0, 0.0), "cached answer");
        assert_eq!(c.get(&emb(1.0, 0.0)).as_deref(), Some("cached answer"));
        assert_eq!(c.stats().hits, 1);
    }

    #[test]
    fn miss_below_threshold() {
        let mut c = cache();
        c.put(&emb(1.0, 0.0), "x");
        // Orthogonal → cosine 0 < 0.95 → miss.
        assert!(c.get(&emb(0.0, 1.0)).is_none());
    }

    #[test]
    fn ttl_expiry_drops_entry() {
        let mut c = SemanticCache::new(3, Duration::from_millis(10));
        let t0 = Instant::now();
        c.put_at(&emb(1.0, 0.0), "stale".to_string(), t0);
        let later = t0 + Duration::from_millis(11);
        assert!(c.get_at(&emb(1.0, 0.0), later).is_none(), "entry past TTL");
        assert!(c.is_empty(), "expired entry pruned");
    }

    #[test]
    fn lru_eviction_when_over_capacity() {
        let mut c = SemanticCache::new(2, Duration::from_secs(60));
        let t0 = Instant::now();
        c.put_at(&emb(1.0, 0.0), "a".to_string(), t0);
        c.put_at(
            &emb(0.0, 1.0),
            "b".to_string(),
            t0 + Duration::from_millis(1),
        );
        // Touch "a" so "b" becomes the least-recently-used.
        assert_eq!(
            c.get_at(&emb(1.0, 0.0), t0 + Duration::from_millis(2))
                .as_deref(),
            Some("a")
        );
        // Insert a third → evicts "b".
        c.put_at(
            &emb(0.5, 0.5),
            "cc".to_string(),
            t0 + Duration::from_millis(3),
        );
        assert_eq!(c.len(), 2);
        assert!(
            c.get_at(&emb(0.0, 1.0), t0 + Duration::from_millis(4))
                .is_none()
        );
        assert_eq!(
            c.get_at(&emb(1.0, 0.0), t0 + Duration::from_millis(5))
                .as_deref(),
            Some("a")
        );
    }

    #[test]
    fn clear_empties_entries() {
        let mut c = cache();
        c.put(&emb(1.0, 0.0), "x");
        c.clear();
        assert!(c.is_empty());
    }

    #[test]
    fn stats_track_hits_and_misses() {
        let mut c = cache();
        c.put(&emb(1.0, 0.0), "x");
        c.get(&emb(1.0, 0.0)); // hit
        c.get(&emb(0.0, 1.0)); // miss
        let s = c.stats();
        assert_eq!(s.hits, 1);
        assert_eq!(s.misses, 1);
        assert_eq!(s.entries, 1);
        assert_eq!(s.capacity, 3);
    }
}
