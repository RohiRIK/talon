use crate::{MemoryError, StoredMessage, Summarizer, WorkingMemory, store::MemoryStore};

/// Assembles the context window for one agent turn.
///
/// Order: system prompt → USER.md → MEMORY.md → FTS5 retrievals → recent N messages.
/// Token budget: hard cap at `max_tokens` (caller sets this to 70% of the model's
/// context window). When the recent-message window overflows the budget, the
/// oldest turns are folded into a rolling summary via an injected [`Summarizer`]
/// ([`Self::summarizer`]); without one, they are dropped oldest-first.
pub struct ContextBuilder<'a> {
    store: &'a dyn MemoryStore,
    session_id: &'a str,
    system_prompt: Option<String>,
    user_md: Option<String>,
    memory_md: Option<String>,
    fts_query: Option<String>,
    fts_limit: usize,
    recent_n: usize,
    max_tokens: usize,
    summarizer: Option<&'a dyn Summarizer>,
}

/// Assembled context ready for the LLM.
#[derive(Debug, Clone)]
pub struct BuiltContext {
    /// Full system prompt text (system + USER.md + MEMORY.md + conversation summary).
    pub system: String,
    /// Messages to include in the conversation window, oldest-first.
    pub messages: Vec<StoredMessage>,
    /// Rolling summary of older turns folded out of the window, if any.
    pub summary: Option<String>,
    /// Number of tokens estimated to be in use.
    pub estimated_tokens: usize,
}

impl<'a> ContextBuilder<'a> {
    pub fn new(store: &'a dyn MemoryStore, session_id: &'a str) -> Self {
        Self {
            store,
            session_id,
            system_prompt: None,
            user_md: None,
            memory_md: None,
            fts_query: None,
            fts_limit: 5,
            recent_n: 20,
            max_tokens: 70_000,
            summarizer: None,
        }
    }

    /// Summarize overflow turns instead of dropping them. When the recent window
    /// exceeds the token budget, the oldest turns are folded into a rolling
    /// summary (surfaced in [`BuiltContext::summary`] and appended to `system`).
    pub fn summarizer(mut self, s: &'a dyn Summarizer) -> Self {
        self.summarizer = Some(s);
        self
    }

    pub fn system_prompt(mut self, s: impl Into<String>) -> Self {
        self.system_prompt = Some(s.into());
        self
    }

    pub fn user_md(mut self, s: impl Into<String>) -> Self {
        self.user_md = Some(s.into());
        self
    }

    pub fn memory_md(mut self, s: impl Into<String>) -> Self {
        self.memory_md = Some(s.into());
        self
    }

    pub fn fts_query(mut self, q: impl Into<String>) -> Self {
        self.fts_query = Some(q.into());
        self
    }

    pub fn fts_limit(mut self, n: usize) -> Self {
        self.fts_limit = n;
        self
    }

    pub fn recent_n(mut self, n: usize) -> Self {
        self.recent_n = n;
        self
    }

    /// Hard token budget for the assembled context. Default: 70 000 tokens.
    pub fn max_tokens(mut self, n: usize) -> Self {
        self.max_tokens = n;
        self
    }

    /// Build the context, respecting the token budget.
    pub async fn build(self) -> Result<BuiltContext, MemoryError> {
        let summarizer = self.summarizer;
        // 1. System block.
        let mut system_parts: Vec<String> = Vec::new();
        if let Some(sp) = self.system_prompt {
            system_parts.push(sp);
        }
        if let Some(u) = self.user_md {
            system_parts.push(format!("## User Context\n{u}"));
        }
        if let Some(m) = self.memory_md {
            system_parts.push(format!("## Memory\n{m}"));
        }
        let system = system_parts.join("\n\n");

        // 2. FTS5 retrievals (relevant past context).
        let mut fts_hits: Vec<StoredMessage> = Vec::new();
        if let Some(q) = self.fts_query {
            fts_hits = self.store.search_messages(&q, self.fts_limit).await?;
        }

        // 3. Recent messages for this session.
        let recent = self
            .store
            .recent_messages(self.session_id, self.recent_n)
            .await?;

        // 4. Assemble under token budget (rough estimate: 1 token ≈ 4 chars).
        let mut budget = self.max_tokens;
        let system_tokens = estimate_tokens(&system);
        budget = budget.saturating_sub(system_tokens);

        // FTS hits first (de-dup against recent by id).
        let recent_ids: std::collections::HashSet<i64> = recent.iter().map(|m| m.id).collect();
        let mut messages: Vec<StoredMessage> = Vec::new();

        for hit in fts_hits {
            if recent_ids.contains(&hit.id) {
                continue; // already in recent window
            }
            let tok = estimate_tokens(&hit.content);
            if tok > budget {
                break;
            }
            budget -= tok;
            messages.push(hit);
        }

        // 5. Fit the recent window into the remaining budget. With a summarizer,
        // overflow turns are folded into a rolling summary; without one, the
        // oldest are dropped.
        let (recent_kept, summary) = Self::fit_recent(summarizer, recent, budget).await?;
        messages.extend(recent_kept);

        let mut system = system;
        if let Some(s) = &summary {
            system.push_str("\n\n## Conversation Summary\n");
            system.push_str(s);
        }

        let estimated_tokens = estimate_tokens(&system)
            + messages
                .iter()
                .map(|m| estimate_tokens(&m.content))
                .sum::<usize>();

        Ok(BuiltContext {
            system,
            messages,
            summary,
            estimated_tokens,
        })
    }

    /// Fit the oldest-first `recent` window into `budget`. Returns the kept
    /// messages (oldest-first) and an optional rolling summary of folded turns.
    async fn fit_recent(
        summarizer: Option<&'a dyn Summarizer>,
        recent: Vec<StoredMessage>,
        budget: usize,
    ) -> Result<(Vec<StoredMessage>, Option<String>), MemoryError> {
        let Some(summarizer) = summarizer else {
            // Drop oldest-first: walk newest→oldest while turns fit.
            let mut budget = budget;
            let mut kept: Vec<StoredMessage> = Vec::new();
            for msg in recent.into_iter().rev() {
                let tok = estimate_tokens(&msg.content);
                if tok > budget {
                    break;
                }
                budget -= tok;
                kept.push(msg);
            }
            kept.reverse();
            return Ok((kept, None));
        };

        let mut wm = WorkingMemory::new(budget.max(1));
        for m in &recent {
            wm.push(m.role.clone(), m.content.clone());
        }
        while wm.needs_compaction() {
            if !wm.compact(summarizer).await? {
                break;
            }
        }
        let start = recent.len().saturating_sub(wm.len());
        let kept = recent.into_iter().skip(start).collect();
        Ok((kept, wm.summary().map(str::to_string)))
    }
}

/// Rough token estimate: 1 token ≈ 4 characters (conservative for English text).
fn estimate_tokens(s: &str) -> usize {
    s.len().div_ceil(4)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;

    use crate::{Database, SqliteStore};

    use super::*;

    async fn make_store() -> Arc<SqliteStore> {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        Arc::new(SqliteStore::new(db))
    }

    #[tokio::test]
    async fn empty_store_builds_context_with_no_messages() {
        let store = make_store().await;
        let ctx = ContextBuilder::new(store.as_ref(), "s1")
            .system_prompt("You are Talon.")
            .build()
            .await
            .expect("build");

        assert!(ctx.system.contains("You are Talon."));
        assert!(ctx.messages.is_empty());
        assert!(ctx.estimated_tokens > 0);
    }

    #[tokio::test]
    async fn system_prompt_user_md_memory_md_are_concatenated() {
        let store = make_store().await;
        let ctx = ContextBuilder::new(store.as_ref(), "s1")
            .system_prompt("BASE")
            .user_md("USER INFO")
            .memory_md("MEMORY INFO")
            .build()
            .await
            .expect("build");

        assert!(ctx.system.contains("BASE"));
        assert!(ctx.system.contains("User Context"));
        assert!(ctx.system.contains("USER INFO"));
        assert!(ctx.system.contains("Memory"));
        assert!(ctx.system.contains("MEMORY INFO"));
    }

    #[tokio::test]
    async fn recent_messages_appear_in_context() {
        let store = make_store().await;
        store
            .save_message("s2", "user", "hello")
            .await
            .expect("save");
        store
            .save_message("s2", "assistant", "world")
            .await
            .expect("save");

        let ctx = ContextBuilder::new(store.as_ref(), "s2")
            .build()
            .await
            .expect("build");

        assert_eq!(ctx.messages.len(), 2);
        assert_eq!(ctx.messages[0].content, "hello");
        assert_eq!(ctx.messages[1].content, "world");
    }

    #[tokio::test]
    async fn token_budget_limits_messages() {
        let store = make_store().await;
        // 100 messages × "word " (5 chars ≈ 2 tokens each) = ~200 tokens total.
        for i in 0..100 {
            store
                .save_message("s3", "user", &format!("word {i}"))
                .await
                .expect("save");
        }
        // Budget of 50 tokens should include far fewer than 100 messages.
        let ctx = ContextBuilder::new(store.as_ref(), "s3")
            .max_tokens(50)
            .recent_n(100)
            .build()
            .await
            .expect("build");

        assert!(
            ctx.messages.len() < 100,
            "budget should have dropped older messages, got {}",
            ctx.messages.len()
        );
        assert!(
            ctx.estimated_tokens <= 50,
            "estimated_tokens={}",
            ctx.estimated_tokens
        );
    }

    #[tokio::test]
    async fn fts_hits_deduped_against_recent() {
        let store = make_store().await;
        store
            .save_message("s4", "user", "search me please")
            .await
            .expect("save");

        let ctx = ContextBuilder::new(store.as_ref(), "s4")
            .fts_query("search")
            .recent_n(10)
            .build()
            .await
            .expect("build");

        // The message appears once (in recent), not twice.
        let count = ctx
            .messages
            .iter()
            .filter(|m| m.content == "search me please")
            .count();
        assert_eq!(count, 1, "message should appear exactly once; got {count}");
    }

    #[test]
    fn estimate_tokens_four_chars_per_token() {
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2); // ceil(5/4) = 2
        assert_eq!(estimate_tokens(""), 0);
    }

    use std::{future::Future, pin::Pin};

    /// Deterministic summarizer: emits a fixed marker so tests can detect folding.
    struct StubSummarizer;
    impl Summarizer for StubSummarizer {
        fn summarize<'a>(
            &'a self,
            _transcript: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<String, MemoryError>> + Send + 'a>> {
            Box::pin(async { Ok("folded older turns".to_string()) })
        }
    }

    #[tokio::test]
    async fn summarizer_folds_overflow_instead_of_dropping() {
        let store = make_store().await;
        for i in 0..40 {
            store
                .save_message("sum", "user", &format!("message {i} {}", "x".repeat(40)))
                .await
                .expect("save");
        }

        let ctx = ContextBuilder::new(store.as_ref(), "sum")
            .max_tokens(120)
            .recent_n(40)
            .summarizer(&StubSummarizer)
            .build()
            .await
            .expect("build");

        assert_eq!(ctx.summary.as_deref(), Some("folded older turns"));
        assert!(
            ctx.system.contains("## Conversation Summary"),
            "summary should be appended to the system block"
        );
        assert!(
            ctx.system.contains("folded older turns"),
            "summary text should appear in system"
        );
        // The newest message must survive verbatim.
        assert!(
            ctx.messages
                .iter()
                .any(|m| m.content.contains("message 39")),
            "newest turn must be kept"
        );
    }

    #[tokio::test]
    async fn summarizer_noop_under_budget_leaves_no_summary() {
        let store = make_store().await;
        store
            .save_message("small", "user", "hello")
            .await
            .expect("save");

        let ctx = ContextBuilder::new(store.as_ref(), "small")
            .summarizer(&StubSummarizer)
            .build()
            .await
            .expect("build");

        assert!(ctx.summary.is_none(), "nothing to fold under budget");
        assert!(!ctx.system.contains("Conversation Summary"));
        assert_eq!(ctx.messages.len(), 1);
    }
}
