//! `Deduplicator` — semantic deduplication of memories (task 2.5.6).
//!
//! Before a new memory is stored, its embedding is compared against the nearest
//! existing memory (sqlite-vec KNN). If their cosine similarity meets the
//! threshold (default 0.85) the existing memory is *reinforced* instead of a
//! near-duplicate row being appended; otherwise the new memory is inserted.
//!
//! Embeddings are supplied by the caller (fastembed is wired in at the gateway
//! layer behind the `semantic-search` feature), so this module stays free of an
//! embedding-model dependency and is exercised with deterministic vectors.

use crate::{LtmStore, Memory, MemoryError};

/// Cosine-similarity threshold at or above which two memories are duplicates.
pub const DEFAULT_DEDUP_THRESHOLD: f32 = 0.85;

/// What happened when a memory was offered to the store.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DedupOutcome {
    /// No near-duplicate found; a new memory was inserted with this id.
    Inserted(i64),
    /// A near-duplicate existed; that memory (this id) was reinforced instead.
    Merged { id: i64, similarity: f32 },
}

/// Cosine similarity of two equal-length vectors. Returns 0.0 if either is the
/// zero vector or lengths differ (no meaningful angle).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Semantic deduplicator over an [`LtmStore`].
#[derive(Debug, Clone, Copy)]
pub struct Deduplicator {
    threshold: f32,
}

impl Default for Deduplicator {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_DEDUP_THRESHOLD,
        }
    }
}

impl Deduplicator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Use a custom similarity threshold (clamped to `[0, 1]`).
    pub fn with_threshold(threshold: f32) -> Self {
        Self {
            threshold: threshold.clamp(0.0, 1.0),
        }
    }

    /// Store `mem` with its `embedding`, deduplicating against existing memories.
    /// Reinforces the nearest existing memory when their cosine similarity meets
    /// the threshold; otherwise inserts a new row.
    pub async fn upsert(
        &self,
        store: &LtmStore,
        mem: &Memory,
        embedding: &[f32],
    ) -> Result<DedupOutcome, MemoryError> {
        if let Some((id, _l2)) = store.search_vector(embedding, 1).await?.first().copied()
            && let Some(existing) = store.get_embedding(id).await?
        {
            let similarity = cosine_similarity(embedding, &existing);
            if similarity >= self.threshold {
                store.reinforce(id, mem.importance).await?;
                return Ok(DedupOutcome::Merged { id, similarity });
            }
        }
        let id = store.insert(mem, Some(embedding)).await?;
        Ok(DedupOutcome::Inserted(id))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{Database, MemoryCategory};

    async fn store() -> LtmStore {
        let db = Database::open(":memory:").expect("open");
        db.init_schema().await.expect("schema");
        LtmStore::new(db)
    }

    fn emb(a: f32, b: f32) -> Vec<f32> {
        let mut v = vec![0.0f32; 384];
        v[0] = a;
        v[1] = b;
        v
    }

    fn mem(content: &str, importance: u8) -> Memory {
        Memory::new(content, MemoryCategory::Fact, importance, vec![], vec![])
    }

    #[test]
    fn cosine_identical_is_one() {
        let v = emb(0.6, 0.8);
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn cosine_orthogonal_is_zero() {
        assert!(cosine_similarity(&emb(1.0, 0.0), &emb(0.0, 1.0)).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector_is_zero() {
        assert_eq!(cosine_similarity(&emb(0.0, 0.0), &emb(1.0, 0.0)), 0.0);
    }

    #[test]
    fn cosine_mismatched_lengths_is_zero() {
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0]), 0.0);
    }

    #[tokio::test]
    async fn first_memory_is_inserted() {
        let s = store().await;
        let out = Deduplicator::new()
            .upsert(&s, &mem("alpha", 3), &emb(1.0, 0.0))
            .await
            .expect("upsert");
        assert!(matches!(out, DedupOutcome::Inserted(_)));
    }

    #[tokio::test]
    async fn near_duplicate_merges_and_does_not_append() {
        let s = store().await;
        let d = Deduplicator::new();
        let first = d
            .upsert(&s, &mem("user prefers dark mode", 3), &emb(1.0, 0.0))
            .await
            .expect("first");
        let DedupOutcome::Inserted(first_id) = first else {
            panic!("first should insert");
        };

        // Identical embedding → cosine 1.0 ≥ 0.85 → merge into the same row.
        let out = d
            .upsert(&s, &mem("prefers a dark theme", 5), &emb(1.0, 0.0))
            .await
            .expect("second");
        match out {
            DedupOutcome::Merged { id, similarity } => {
                assert_eq!(id, first_id);
                assert!(similarity >= 0.85);
            }
            DedupOutcome::Inserted(_) => panic!("near-duplicate must merge, not append"),
        }

        // Merge reinforced importance to the higher value, not a second row.
        assert_eq!(
            s.get(first_id)
                .await
                .expect("get")
                .expect("present")
                .importance,
            5
        );
    }

    #[tokio::test]
    async fn distinct_memory_is_inserted_not_merged() {
        let s = store().await;
        let d = Deduplicator::new();
        d.upsert(&s, &mem("likes tabs", 3), &emb(1.0, 0.0))
            .await
            .expect("first");
        // Orthogonal embedding → cosine 0 < 0.85 → new row.
        let out = d
            .upsert(&s, &mem("uses zed editor", 3), &emb(0.0, 1.0))
            .await
            .expect("second");
        assert!(matches!(out, DedupOutcome::Inserted(_)));
    }

    #[tokio::test]
    async fn threshold_controls_merge_decision() {
        let s = store().await;
        // ~0.707 cosine between [1,0] and [1,1].
        let strict = Deduplicator::with_threshold(0.9);
        strict
            .upsert(&s, &mem("a", 3), &emb(1.0, 0.0))
            .await
            .expect("first");
        let out = strict
            .upsert(&s, &mem("b", 3), &emb(1.0, 1.0))
            .await
            .expect("second");
        assert!(
            matches!(out, DedupOutcome::Inserted(_)),
            "0.707 < 0.9 threshold → insert"
        );
    }
}
