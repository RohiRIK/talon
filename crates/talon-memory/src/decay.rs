//! `DecayEngine` — time-based memory decay (task 2.5.10).
//!
//! Memories carry a `decay_score` in `[0, 1]` that erodes with age. The engine
//! recomputes every memory's score from how long ago it was last accessed using
//! an exponential half-life: a memory untouched for one half-life sits at 0.5,
//! two half-lives at 0.25, and so on. Accessing or reinforcing a memory refreshes
//! its `accessed_at`, so live memories stay near 1.0 while forgotten ones fade.
//!
//! This is a periodic maintenance pass (run once per day, not per query) — the
//! batch update commits in a single transaction.

use crate::{LtmStore, MemoryError};

/// Default half-life: a memory loses half its score after a month untouched.
pub const DEFAULT_HALF_LIFE_DAYS: f64 = 30.0;

const SECONDS_PER_DAY: f64 = 86_400.0;

/// Exponential decay factor in `[0, 1]` for a memory last accessed `age_seconds`
/// ago. `0.5^(age_days / half_life_days)`. Future timestamps (negative age) clamp
/// to 1.0 — a memory is never "fresher than new".
pub fn decay_factor(age_seconds: i64, half_life_days: f64) -> f32 {
    if half_life_days <= 0.0 {
        return 1.0;
    }
    let age_days = age_seconds.max(0) as f64 / SECONDS_PER_DAY;
    0.5f64.powf(age_days / half_life_days).clamp(0.0, 1.0) as f32
}

/// Periodic decay pass over an [`LtmStore`].
#[derive(Debug, Clone, Copy)]
pub struct DecayEngine {
    half_life_days: f64,
}

impl Default for DecayEngine {
    fn default() -> Self {
        Self {
            half_life_days: DEFAULT_HALF_LIFE_DAYS,
        }
    }
}

impl DecayEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Use a custom half-life (days).
    pub fn with_half_life_days(half_life_days: f64) -> Self {
        Self { half_life_days }
    }

    /// Recompute `decay_score` for every memory as of `now_unix`. Returns the
    /// number of memories updated.
    pub async fn run(&self, store: &LtmStore, now_unix: i64) -> Result<usize, MemoryError> {
        store.apply_decay(self.half_life_days, now_unix).await
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{Database, Memory, MemoryCategory};

    async fn store() -> LtmStore {
        let db = Database::open(":memory:").expect("open");
        db.init_schema().await.expect("schema");
        LtmStore::new(db)
    }

    const HALF_LIFE_SECS: i64 = (DEFAULT_HALF_LIFE_DAYS as i64) * 86_400;

    #[test]
    fn fresh_memory_has_full_score() {
        assert!((decay_factor(0, DEFAULT_HALF_LIFE_DAYS) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn one_half_life_halves_score() {
        let f = decay_factor(HALF_LIFE_SECS, DEFAULT_HALF_LIFE_DAYS);
        assert!((f - 0.5).abs() < 1e-4, "got {f}");
    }

    #[test]
    fn two_half_lives_quarter_score() {
        let f = decay_factor(2 * HALF_LIFE_SECS, DEFAULT_HALF_LIFE_DAYS);
        assert!((f - 0.25).abs() < 1e-4, "got {f}");
    }

    #[test]
    fn future_timestamp_clamps_to_one() {
        assert_eq!(decay_factor(-1000, DEFAULT_HALF_LIFE_DAYS), 1.0);
    }

    #[tokio::test]
    async fn run_decays_stale_memory() {
        let s = store().await;
        let id = s
            .insert(
                &Memory::new("old fact", MemoryCategory::Fact, 3, vec![], vec![]),
                None,
            )
            .await
            .expect("insert");

        let accessed = s.get(id).await.expect("get").expect("present").accessed_at;
        // One half-life after this memory was last accessed.
        let updated = DecayEngine::new()
            .run(&s, accessed + HALF_LIFE_SECS)
            .await
            .expect("run");
        assert_eq!(updated, 1);

        let score = s.get(id).await.expect("get").expect("present").decay_score;
        assert!((score - 0.5).abs() < 1e-3, "expected ~0.5, got {score}");
    }

    #[tokio::test]
    async fn run_leaves_fresh_memory_near_full() {
        let s = store().await;
        let id = s
            .insert(
                &Memory::new("recent", MemoryCategory::Fact, 3, vec![], vec![]),
                None,
            )
            .await
            .expect("insert");
        let accessed = s.get(id).await.expect("get").expect("present").accessed_at;
        DecayEngine::new().run(&s, accessed).await.expect("run");
        let score = s.get(id).await.expect("get").expect("present").decay_score;
        assert!((score - 1.0).abs() < 1e-6);
    }
}
