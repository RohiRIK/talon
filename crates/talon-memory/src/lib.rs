use deadpool_sqlite::{Config, Pool, Runtime};
use thiserror::Error;

/// Type #3 — SQLite connection pool wrapper.
///
/// Rule: `rusqlite::Connection` is `!Send`. This struct exposes only the pool,
/// never a raw `Connection`. All DB operations use `.interact(|conn| { ... }).await?`
/// so the connection never crosses an await point. See ADR 0004.
///
/// Full schema (FTS5, sessions, messages, tools) added in Phase 2.
/// LanceDB long-term memory integration added in Phase 2.5.
#[derive(Debug, Clone)]
pub struct Database {
    pool: Pool,
}

impl Database {
    /// Open (or create) the SQLite database at the given path.
    pub fn open(path: &str) -> Result<Self, DatabaseError> {
        let pool = Config::new(path)
            .create_pool(Runtime::Tokio1)
            .map_err(|e| DatabaseError::Pool(e.to_string()))?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Create sessions and messages tables if they do not exist.
    /// Safe to call on every startup (uses IF NOT EXISTS).
    pub async fn init_schema(&self) -> Result<(), DatabaseError> {
        self.pool
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e.to_string()))?
            .interact(|conn| {
                conn.execute_batch(
                    "PRAGMA journal_mode=WAL;
                     CREATE TABLE IF NOT EXISTS sessions (
                         id         TEXT PRIMARY KEY,
                         created_at INTEGER NOT NULL
                     );
                     CREATE TABLE IF NOT EXISTS messages (
                         id         INTEGER PRIMARY KEY AUTOINCREMENT,
                         session_id TEXT    NOT NULL,
                         role       TEXT    NOT NULL,
                         content    TEXT    NOT NULL,
                         created_at INTEGER NOT NULL
                     );",
                )
            })
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .map_err(|e: rusqlite::Error| DatabaseError::Query(e.to_string()))
    }

    /// Persist a single message to the messages table.
    pub async fn save_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
    ) -> Result<(), DatabaseError> {
        let session_id = session_id.to_string();
        let role = role.to_string();
        let content = content.to_string();
        self.pool
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e.to_string()))?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO messages (session_id, role, content, created_at)
                     VALUES (?1, ?2, ?3, unixepoch())",
                    rusqlite::params![session_id, role, content],
                )
            })
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .map(|_| ())
            .map_err(|e: rusqlite::Error| DatabaseError::Query(e.to_string()))
    }
}

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("failed to create connection pool: {0}")]
    Pool(String),
    #[error("query failed: {0}")]
    Query(String),
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn database_open_in_memory_succeeds() {
        let db = Database::open(":memory:");
        assert!(db.is_ok(), "in-memory DB should open without error");
    }

    #[test]
    fn database_pool_is_accessible() {
        let db = Database::open(":memory:").expect("open");
        let _pool = db.pool();
    }

    #[test]
    fn database_error_pool_display() {
        let err = DatabaseError::Pool("bad path".to_string());
        assert!(err.to_string().contains("bad path"));
    }

    #[tokio::test]
    async fn init_schema_creates_tables() {
        let db = Database::open(":memory:").expect("open");
        db.init_schema().await.expect("init_schema");
    }

    #[tokio::test]
    async fn init_schema_is_idempotent() {
        let db = Database::open(":memory:").expect("open");
        db.init_schema().await.expect("first call");
        db.init_schema().await.expect("second call — idempotent");
    }

    #[tokio::test]
    async fn save_message_persists_and_is_readable() {
        let db = Database::open(":memory:").expect("open");
        db.init_schema().await.expect("schema");
        db.save_message("session-1", "user", "hello").await.expect("save");

        let count: i64 = db
            .pool()
            .get()
            .await
            .expect("pool")
            .interact(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM messages WHERE session_id = 'session-1'",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .expect("interact")
            .expect("query");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn save_multiple_messages_maintains_order() {
        let db = Database::open(":memory:").expect("open");
        db.init_schema().await.expect("schema");
        db.save_message("s1", "user", "first").await.expect("1");
        db.save_message("s1", "assistant", "second").await.expect("2");

        let roles: Vec<String> = db
            .pool()
            .get()
            .await
            .expect("pool")
            .interact(|conn| {
                let mut stmt = conn
                    .prepare("SELECT role FROM messages WHERE session_id='s1' ORDER BY id")
                    .expect("prepare");
                stmt.query_map([], |row| row.get(0))
                    .expect("query")
                    .collect::<Result<Vec<String>, _>>()
            })
            .await
            .expect("interact")
            .expect("collect");
        assert_eq!(roles, ["user", "assistant"]);
    }
}
