//! `Promoter` — post-session promotion of working-memory facts to LTM (task 2.5.7).
//!
//! At session end the [`crate::FactExtractor`] turns the transcript into candidate
//! [`Memory`] facts. The `Promoter` keeps only the high-importance ones, embeds
//! each, and writes it into the `memories` table through the [`Deduplicator`] so
//! near-duplicates reinforce an existing memory rather than appending a new row.
//!
//! The embedding model is injected via the [`Embedder`] trait so `talon-memory`
//! stays free of a fastembed dependency (same pattern as [`crate::Summarizer`]
//! and [`crate::FactCompleter`]); a deterministic stub is used in tests.

use std::{future::Future, pin::Pin};

use crate::{DedupOutcome, Deduplicator, LtmStore, Memory, MemoryError};

/// Produces an embedding vector for a piece of text. Implemented by a fastembed-
/// backed type outside this crate; a deterministic stub is used in tests.
pub trait Embedder: Send + Sync {
    fn embed<'a>(
        &'a self,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<f32>, MemoryError>> + Send + 'a>>;
}

/// Facts below this importance are not promoted to long-term memory.
pub const DEFAULT_PROMOTION_MIN_IMPORTANCE: u8 = 4;

/// Tally of what a promotion run did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PromotionReport {
    /// New memories written.
    pub inserted: usize,
    /// Facts that reinforced an existing near-duplicate memory.
    pub merged: usize,
    /// Facts dropped for being below the importance threshold.
    pub skipped: usize,
}

impl PromotionReport {
    /// Total facts that reached long-term memory (inserted or merged).
    pub fn promoted(&self) -> usize {
        self.inserted + self.merged
    }
}

/// Promotes high-importance facts into the long-term [`LtmStore`].
#[derive(Debug, Clone, Copy)]
pub struct Promoter {
    min_importance: u8,
    dedup: Deduplicator,
}

impl Default for Promoter {
    fn default() -> Self {
        Self {
            min_importance: DEFAULT_PROMOTION_MIN_IMPORTANCE,
            dedup: Deduplicator::new(),
        }
    }
}

impl Promoter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Promote facts scoring at or above `min_importance`.
    pub fn with_min_importance(min_importance: u8) -> Self {
        Self {
            min_importance,
            dedup: Deduplicator::new(),
        }
    }

    /// Promote `facts` into long-term memory. Each surviving fact is embedded and
    /// deduplicated; below-threshold facts are skipped. Returns a [`PromotionReport`].
    pub async fn promote<I>(
        &self,
        facts: I,
        store: &LtmStore,
        embedder: &dyn Embedder,
    ) -> Result<PromotionReport, MemoryError>
    where
        I: IntoIterator<Item = Memory>,
    {
        let mut report = PromotionReport::default();
        for fact in facts {
            if fact.importance < self.min_importance {
                report.skipped += 1;
                continue;
            }
            let embedding = embedder.embed(&fact.content).await?;
            match self.dedup.upsert(store, &fact, &embedding).await? {
                DedupOutcome::Inserted(_) => report.inserted += 1,
                DedupOutcome::Merged { .. } => report.merged += 1,
            }
        }
        Ok(report)
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

    /// Deterministic embedder: maps the first byte of `text` onto one axis so
    /// identical content collides (cosine 1.0) and different content separates.
    struct StubEmbedder;
    impl Embedder for StubEmbedder {
        fn embed<'a>(
            &'a self,
            text: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<f32>, MemoryError>> + Send + 'a>> {
            let first = text.bytes().next().unwrap_or(0);
            let mut v = vec![0.0f32; 384];
            v[first as usize % 384] = 1.0;
            Box::pin(async move { Ok(v) })
        }
    }

    fn mem(content: &str, importance: u8) -> Memory {
        Memory::new(content, MemoryCategory::Fact, importance, vec![], vec![])
    }

    #[tokio::test]
    async fn promotes_only_high_importance_facts() {
        let s = store().await;
        let facts = vec![mem("Alpha", 5), mem("beta", 2), mem("Gamma", 4)];
        let report = Promoter::new()
            .promote(facts, &s, &StubEmbedder)
            .await
            .expect("promote");
        assert_eq!(report.inserted, 2, "importance 5 and 4 promoted");
        assert_eq!(report.skipped, 1, "importance 2 skipped");
        assert_eq!(report.merged, 0);
    }

    #[tokio::test]
    async fn duplicate_facts_merge_not_append() {
        let s = store().await;
        // Same first byte → identical stub embedding → dedup merges the second.
        let facts = vec![mem("dark mode preferred", 5), mem("dark theme please", 5)];
        let report = Promoter::new()
            .promote(facts, &s, &StubEmbedder)
            .await
            .expect("promote");
        assert_eq!(report.inserted, 1);
        assert_eq!(report.merged, 1);
        assert_eq!(report.promoted(), 2);
    }

    #[tokio::test]
    async fn custom_threshold_changes_what_promotes() {
        let s = store().await;
        let facts = vec![mem("x", 3), mem("y", 2)];
        let report = Promoter::with_min_importance(3)
            .promote(facts, &s, &StubEmbedder)
            .await
            .expect("promote");
        assert_eq!(report.inserted, 1);
        assert_eq!(report.skipped, 1);
    }

    #[tokio::test]
    async fn empty_facts_promote_nothing() {
        let s = store().await;
        let report = Promoter::new()
            .promote(Vec::new(), &s, &StubEmbedder)
            .await
            .expect("promote");
        assert_eq!(report, PromotionReport::default());
        assert_eq!(report.promoted(), 0);
    }
}
