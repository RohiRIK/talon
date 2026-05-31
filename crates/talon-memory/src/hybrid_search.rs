//! `HybridSearch` — FTS5 ⊕ sqlite-vec retrieval fused with RRF (task 2.5.8).
//!
//! Keyword relevance (FTS5 BM25) and semantic similarity (sqlite-vec KNN) each
//! produce a ranked list of memory ids. Reciprocal Rank Fusion (RRF) combines
//! them by rank — not by raw score — so the two incomparable scales (BM25 vs.
//! L2 distance) never need normalising. A memory that ranks well in *either*
//! arm surfaces; one that ranks well in *both* rises to the top.

use std::collections::HashMap;

use crate::{LtmStore, Memory, MemoryError};

/// RRF damping constant. 60 is the value from the original Cormack et al. paper
/// and the de-facto default; larger flattens the rank contribution.
pub const RRF_K: f64 = 60.0;

/// Fuse several ranked id-lists into one, scored by Reciprocal Rank Fusion.
/// Each list is ordered best-first; an id's score is `Σ 1/(k + rank)` across the
/// lists it appears in (rank is 1-based). Returns `(id, score)` best-first, with
/// ascending id as a deterministic tie-break.
pub fn reciprocal_rank_fusion(rankings: &[Vec<i64>], k: f64) -> Vec<(i64, f64)> {
    let mut scores: HashMap<i64, f64> = HashMap::new();
    for list in rankings {
        for (idx, id) in list.iter().enumerate() {
            let rank = (idx + 1) as f64;
            *scores.entry(*id).or_insert(0.0) += 1.0 / (k + rank);
        }
    }
    let mut fused: Vec<(i64, f64)> = scores.into_iter().collect();
    fused.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    fused
}

/// Hybrid keyword-plus-vector retrieval over an [`LtmStore`].
#[derive(Debug, Clone, Copy)]
pub struct HybridSearch {
    k: f64,
}

impl Default for HybridSearch {
    fn default() -> Self {
        Self { k: RRF_K }
    }
}

impl HybridSearch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Use a custom RRF damping constant.
    pub fn with_k(k: f64) -> Self {
        Self { k }
    }

    /// Retrieve the top `limit` memories for `query` (keyword) fused with
    /// `embedding` (semantic). Each arm contributes a candidate pool wider than
    /// `limit` so fusion has room to re-rank.
    pub async fn search(
        &self,
        store: &LtmStore,
        query: &str,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<Memory>, MemoryError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let pool = (limit * 4).max(10);

        let text_hits = store.search_text(query, pool).await?;
        let vec_hits = store.search_vector(embedding, pool).await?;

        let text_ids: Vec<i64> = text_hits.iter().map(|m| m.id).collect();
        let vec_ids: Vec<i64> = vec_hits.iter().map(|(id, _)| *id).collect();
        let fused = reciprocal_rank_fusion(&[text_ids, vec_ids], self.k);

        // Reuse memories already materialised by the keyword arm; fetch the rest.
        let mut by_id: HashMap<i64, Memory> = text_hits.into_iter().map(|m| (m.id, m)).collect();

        let mut results = Vec::with_capacity(limit);
        for (id, _score) in fused.into_iter().take(limit) {
            if let Some(m) = by_id.remove(&id) {
                results.push(m);
            } else if let Some(m) = store.get(id).await? {
                results.push(m);
            }
        }
        Ok(results)
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

    fn mem(content: &str) -> Memory {
        Memory::new(content, MemoryCategory::Fact, 3, vec![], vec![])
    }

    #[test]
    fn rrf_rewards_agreement_across_lists() {
        // id 2 is rank-1 in list A and rank-2 in list B → should win overall.
        let a = vec![2, 1, 3];
        let b = vec![3, 2, 1];
        let fused = reciprocal_rank_fusion(&[a, b], RRF_K);
        assert_eq!(fused[0].0, 2);
    }

    #[test]
    fn rrf_sums_scores_for_shared_ids() {
        let fused = reciprocal_rank_fusion(&[vec![5], vec![5]], RRF_K);
        assert_eq!(fused.len(), 1);
        let expected = 2.0 / (RRF_K + 1.0);
        assert!((fused[0].1 - expected).abs() < 1e-9);
    }

    #[test]
    fn rrf_empty_input_is_empty() {
        assert!(reciprocal_rank_fusion(&[], RRF_K).is_empty());
    }

    #[test]
    fn rrf_tie_breaks_on_ascending_id() {
        // Both rank-1 in their own single-item list → equal score, lower id first.
        let fused = reciprocal_rank_fusion(&[vec![9], vec![4]], RRF_K);
        assert_eq!(fused[0].0, 4);
        assert_eq!(fused[1].0, 9);
    }

    #[tokio::test]
    async fn hybrid_surfaces_memory_strong_in_both_arms() {
        let s = store().await;
        // "dark mode" matches the query text AND sits nearest the query vector.
        let target = s
            .insert(&mem("user prefers dark mode"), Some(&emb(1.0, 0.0)))
            .await
            .expect("insert target");
        // Keyword-only match, far in vector space.
        s.insert(&mem("dark chocolate recipe"), Some(&emb(0.0, 1.0)))
            .await
            .expect("insert distractor");
        // Vector-near but no keyword overlap.
        s.insert(&mem("likes tabs over spaces"), Some(&emb(0.95, 0.05)))
            .await
            .expect("insert distractor2");

        let hits = HybridSearch::new()
            .search(&s, "dark mode", &emb(1.0, 0.0), 3)
            .await
            .expect("search");
        assert_eq!(hits[0].id, target, "memory strong in both arms ranks first");
    }

    #[tokio::test]
    async fn hybrid_includes_vector_only_match() {
        let s = store().await;
        s.insert(&mem("apples and oranges"), Some(&emb(0.0, 1.0)))
            .await
            .expect("insert");
        // No keyword overlap with the query, but it is the nearest vector.
        let vec_only = s
            .insert(&mem("citrus fruit notes"), Some(&emb(1.0, 0.0)))
            .await
            .expect("insert");

        let hits = HybridSearch::new()
            .search(&s, "zzzznomatch", &emb(1.0, 0.0), 5)
            .await
            .expect("search");
        assert!(
            hits.iter().any(|m| m.id == vec_only),
            "a vector-only match must still be retrieved and materialised"
        );
    }

    #[tokio::test]
    async fn hybrid_zero_limit_returns_empty() {
        let s = store().await;
        s.insert(&mem("something"), Some(&emb(1.0, 0.0)))
            .await
            .expect("insert");
        let hits = HybridSearch::new()
            .search(&s, "something", &emb(1.0, 0.0), 0)
            .await
            .expect("search");
        assert!(hits.is_empty());
    }
}
