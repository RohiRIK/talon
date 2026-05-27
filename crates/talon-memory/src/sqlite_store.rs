use std::{future::Future, pin::Pin, sync::Arc};

use crate::{Database, MemoryError, StoredMessage, store::MemoryStore};

/// `MemoryStore` implementation backed by SQLite FTS5.
#[derive(Clone)]
pub struct SqliteStore {
    db: Arc<Database>,
}

impl SqliteStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

impl MemoryStore for SqliteStore {
    fn save_message<'a>(
        &'a self,
        session_id: &'a str,
        role: &'a str,
        content: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), MemoryError>> + Send + 'a>> {
        Box::pin(self.db.save_message(session_id, role, content))
    }

    fn search_messages<'a>(
        &'a self,
        query: &'a str,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<StoredMessage>, MemoryError>> + Send + 'a>> {
        Box::pin(self.db.search_messages(query, limit))
    }

    fn recent_messages<'a>(
        &'a self,
        session_id: &'a str,
        n: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<StoredMessage>, MemoryError>> + Send + 'a>> {
        Box::pin(self.db.recent_messages(session_id, n))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    async fn make_store() -> SqliteStore {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        SqliteStore::new(db)
    }

    #[tokio::test]
    async fn save_and_retrieve_recent_message() {
        let store = make_store().await;
        store.save_message("s1", "user", "hello FTS5").await.expect("save");
        let rows = store.recent_messages("s1", 5).await.expect("recent");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].content, "hello FTS5");
    }

    #[tokio::test]
    async fn search_returns_matching_message() {
        let store = make_store().await;
        store.save_message("s2", "user", "Rust is fast").await.expect("save");
        store.save_message("s2", "assistant", "hello world").await.expect("save");

        let results = store.search_messages("fast", 10).await.expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "Rust is fast");
    }

    #[tokio::test]
    async fn search_returns_empty_on_no_match() {
        let store = make_store().await;
        store.save_message("s3", "user", "something else").await.expect("save");
        let results = store.search_messages("xyzzy", 10).await.expect("search");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn recent_messages_oldest_first_ordering() {
        let store = make_store().await;
        store.save_message("s4", "user", "first").await.expect("1");
        store.save_message("s4", "assistant", "second").await.expect("2");
        let rows = store.recent_messages("s4", 10).await.expect("recent");
        assert_eq!(rows[0].content, "first");
        assert_eq!(rows[1].content, "second");
    }

    #[tokio::test]
    async fn arc_dyn_store_dispatches_correctly() {
        let store: Arc<dyn MemoryStore> = Arc::new(make_store().await);
        store.save_message("s5", "user", "dyn dispatch").await.expect("save");
        let rows = store.recent_messages("s5", 5).await.expect("recent");
        assert_eq!(rows.len(), 1);
    }
}
