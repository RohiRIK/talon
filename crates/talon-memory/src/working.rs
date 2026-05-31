//! `WorkingMemory` — token-budgeted short-term window (task 2.5.4).
//!
//! Holds the recent conversation turns. When the estimated token count exceeds
//! the budget, the oldest turns are folded into a rolling `summary` via an
//! injected [`Summarizer`], and only the most recent turns are kept verbatim.
//! This is the "working" half of the claude-ltm two-tier pattern (doc #67); the
//! "long-term" half is the [`crate::LtmStore`].
//!
//! The LLM is not a dependency of this crate: summarization is abstracted behind
//! the [`Summarizer`] trait so the concrete LLM-backed implementation can be
//! wired in at the gateway/core layer (task 2.5.11) without `talon-memory`
//! depending on `talon-llm`.

use std::{future::Future, pin::Pin};

use crate::MemoryError;

/// Rough per-message structural overhead (role marker, separators) in tokens.
const ROLE_OVERHEAD: usize = 4;

/// Estimate token count for a string. Heuristic: ~4 chars per token. Good enough
/// for budgeting; exact tokenization is the provider's job at call time.
pub fn estimate_tokens(s: &str) -> usize {
    s.chars().count().div_ceil(4)
}

/// Summarizes a transcript into a compact form. Implemented by an LLM-backed
/// type outside this crate; a deterministic stub is used in tests.
pub trait Summarizer: Send + Sync {
    fn summarize<'a>(
        &'a self,
        transcript: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, MemoryError>> + Send + 'a>>;
}

/// A single conversation turn held in working memory.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub role: String,
    pub content: String,
}

impl Turn {
    fn tokens(&self) -> usize {
        estimate_tokens(&self.content) + ROLE_OVERHEAD
    }
}

/// Token-budgeted window of recent turns plus a rolling summary of older ones.
#[derive(Debug, Clone)]
pub struct WorkingMemory {
    budget_tokens: usize,
    summary: Option<String>,
    turns: Vec<Turn>,
}

impl WorkingMemory {
    /// Create a working memory with the given token budget.
    pub fn new(budget_tokens: usize) -> Self {
        Self {
            budget_tokens,
            summary: None,
            turns: Vec::new(),
        }
    }

    /// Append a turn.
    pub fn push(&mut self, role: impl Into<String>, content: impl Into<String>) {
        self.turns.push(Turn {
            role: role.into(),
            content: content.into(),
        });
    }

    /// Number of verbatim turns currently held (excludes the summary).
    pub fn len(&self) -> usize {
        self.turns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    /// Current rolling summary, if any older turns have been compacted.
    pub fn summary(&self) -> Option<&str> {
        self.summary.as_deref()
    }

    /// Estimated total token footprint: summary + all verbatim turns.
    pub fn estimated_tokens(&self) -> usize {
        let summary = self.summary.as_deref().map_or(0, estimate_tokens);
        summary + self.turns.iter().map(Turn::tokens).sum::<usize>()
    }

    /// Whether the footprint currently exceeds the budget.
    pub fn needs_compaction(&self) -> bool {
        self.estimated_tokens() > self.budget_tokens
    }

    /// Render summary + verbatim turns as a single transcript for prompting.
    pub fn transcript(&self) -> String {
        let mut out = String::new();
        if let Some(s) = &self.summary {
            out.push_str("[summary] ");
            out.push_str(s);
            out.push_str("\n\n");
        }
        for t in &self.turns {
            out.push_str(&t.role);
            out.push_str(": ");
            out.push_str(&t.content);
            out.push('\n');
        }
        out
    }

    /// Fold the oldest turns into the rolling summary until the footprint fits
    /// the budget. Keeps the most recent turns (up to half the budget) verbatim.
    ///
    /// Returns `true` if compaction ran, `false` if it was unnecessary or could
    /// not shrink further (a single turn larger than the keep-budget).
    pub async fn compact(&mut self, summarizer: &dyn Summarizer) -> Result<bool, MemoryError> {
        if !self.needs_compaction() {
            return Ok(false);
        }

        // Walk back from the newest turn, keeping turns while they fit half the
        // budget. `split` is the index of the first kept turn.
        let keep_budget = self.budget_tokens / 2;
        let mut kept = 0usize;
        let mut split = self.turns.len();
        for (i, turn) in self.turns.iter().enumerate().rev() {
            let t = turn.tokens();
            if kept + t > keep_budget {
                break;
            }
            kept += t;
            split = i;
        }

        if split == 0 {
            // Newest turn alone exceeds the keep-budget — nothing older to fold.
            return Ok(false);
        }

        let mut transcript = String::new();
        if let Some(prev) = &self.summary {
            transcript.push_str(prev);
            transcript.push_str("\n\n");
        }
        for turn in &self.turns[..split] {
            transcript.push_str(&turn.role);
            transcript.push_str(": ");
            transcript.push_str(&turn.content);
            transcript.push('\n');
        }

        let summary = summarizer.summarize(&transcript).await?;
        self.summary = Some(summary);
        self.turns.drain(..split);
        Ok(true)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Deterministic summarizer: records the input length as a short marker.
    struct StubSummarizer;
    impl Summarizer for StubSummarizer {
        fn summarize<'a>(
            &'a self,
            transcript: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<String, MemoryError>> + Send + 'a>> {
            let n = transcript.lines().count();
            Box::pin(async move { Ok(format!("summary of {n} lines")) })
        }
    }

    fn long(words: usize) -> String {
        vec!["lorem"; words].join(" ")
    }

    #[test]
    fn estimate_tokens_grows_with_length() {
        assert!(estimate_tokens("a much longer string here") > estimate_tokens("hi"));
    }

    #[test]
    fn push_increments_len() {
        let mut wm = WorkingMemory::new(1000);
        assert!(wm.is_empty());
        wm.push("user", "hello");
        wm.push("assistant", "hi");
        assert_eq!(wm.len(), 2);
    }

    #[test]
    fn needs_compaction_tracks_budget() {
        let mut wm = WorkingMemory::new(20);
        wm.push("user", "short");
        assert!(!wm.needs_compaction());
        wm.push("assistant", long(200));
        assert!(wm.needs_compaction());
    }

    #[tokio::test]
    async fn compact_is_noop_under_budget() {
        let mut wm = WorkingMemory::new(10_000);
        wm.push("user", "hello");
        let ran = wm.compact(&StubSummarizer).await.expect("compact");
        assert!(!ran);
        assert!(wm.summary().is_none());
        assert_eq!(wm.len(), 1);
    }

    #[tokio::test]
    async fn compact_summarizes_and_shrinks() {
        let mut wm = WorkingMemory::new(80);
        for i in 0..12 {
            wm.push("user", format!("message number {i} {}", long(8)));
        }
        assert!(wm.needs_compaction());
        let before = wm.len();

        let ran = wm.compact(&StubSummarizer).await.expect("compact");
        assert!(ran);
        assert!(wm.summary().is_some(), "a rolling summary should be set");
        assert!(wm.len() < before, "older turns should be dropped");
        assert!(
            wm.estimated_tokens() <= 80 || wm.len() == 1,
            "footprint should fit budget after compaction"
        );
    }

    #[tokio::test]
    async fn compact_keeps_most_recent_turn() {
        let mut wm = WorkingMemory::new(80);
        for i in 0..12 {
            wm.push("user", format!("msg {i} {}", long(8)));
        }
        wm.compact(&StubSummarizer).await.expect("compact");
        // The newest turn must survive verbatim.
        let last = wm.transcript();
        assert!(last.contains("msg 11"), "newest turn must be retained");
    }

    #[tokio::test]
    async fn transcript_includes_summary_after_compaction() {
        let mut wm = WorkingMemory::new(60);
        for i in 0..10 {
            wm.push("assistant", format!("reply {i} {}", long(6)));
        }
        wm.compact(&StubSummarizer).await.expect("compact");
        assert!(wm.transcript().starts_with("[summary]"));
    }
}
