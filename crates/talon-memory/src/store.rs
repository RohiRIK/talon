use std::{future::Future, pin::Pin};

use crate::{MemoryError, StoredMessage};

/// Trait for message persistence and retrieval.
///
/// Implementations: `SqliteStore` (FTS5, synchronous).
/// Future: `LanceStore` (vector + hybrid search, Phase 2.5).
pub trait MemoryStore: Send + Sync {
    /// Persist a single message.
    fn save_message<'a>(
        &'a self,
        session_id: &'a str,
        role: &'a str,
        content: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), MemoryError>> + Send + 'a>>;

    /// Return up to `limit` FTS5-ranked results matching `query`.
    fn search_messages<'a>(
        &'a self,
        query: &'a str,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<StoredMessage>, MemoryError>> + Send + 'a>>;

    /// Return the `n` most recent messages for a session, oldest-first.
    fn recent_messages<'a>(
        &'a self,
        session_id: &'a str,
        n: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<StoredMessage>, MemoryError>> + Send + 'a>>;
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // Minimal stub to verify dyn dispatch compiles.
    struct StubStore;
    impl MemoryStore for StubStore {
        fn save_message<'a>(
            &'a self,
            _session_id: &'a str,
            _role: &'a str,
            _content: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<(), MemoryError>> + Send + 'a>> {
            Box::pin(async { Ok(()) })
        }
        fn search_messages<'a>(
            &'a self,
            _query: &'a str,
            _limit: usize,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<StoredMessage>, MemoryError>> + Send + 'a>>
        {
            Box::pin(async { Ok(vec![]) })
        }
        fn recent_messages<'a>(
            &'a self,
            _session_id: &'a str,
            _n: usize,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<StoredMessage>, MemoryError>> + Send + 'a>>
        {
            Box::pin(async { Ok(vec![]) })
        }
    }

    #[tokio::test]
    async fn arc_dyn_memory_store_is_constructible() {
        let store: Arc<dyn MemoryStore> = Arc::new(StubStore);
        store
            .save_message("s", "user", "hello")
            .await
            .expect("save");
    }

    #[tokio::test]
    async fn stub_search_returns_empty() {
        let store = StubStore;
        let results = store.search_messages("anything", 10).await.expect("search");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn stub_recent_returns_empty() {
        let store = StubStore;
        let results = store.recent_messages("s", 5).await.expect("recent");
        assert!(results.is_empty());
    }
}
