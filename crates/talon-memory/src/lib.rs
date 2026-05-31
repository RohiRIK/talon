pub mod context;
pub mod dedup;
pub mod error;
pub mod facts;
pub mod files;
pub mod ltm;
pub mod migrations;
pub mod sqlite_store;
pub mod store;
pub mod working;

pub use context::{BuiltContext, ContextBuilder};
pub use dedup::{DedupOutcome, Deduplicator, cosine_similarity};
pub use error::MemoryError;
pub use facts::{FactCompleter, FactExtractor};
pub use files::{MemoryMd, UserMd};
pub use ltm::{LtmStore, Memory, MemoryCategory};
pub use sqlite_store::SqliteStore;
pub use store::MemoryStore;
pub use working::{Summarizer, WorkingMemory};

use std::sync::Once;

use deadpool_sqlite::{Config, Pool, Runtime};
use rusqlite::params;
use thiserror::Error;

/// Register the `sqlite-vec` extension exactly once, as a SQLite *auto-extension*
/// so every connection opened afterwards — including pooled ones — gets the
/// `vec0` virtual table. Statically compiled in (no external `.so`). See ADR 0008.
static VEC_INIT: Once = Once::new();

pub(crate) fn register_sqlite_vec() {
    VEC_INIT.call_once(|| {
        // Safety: `sqlite3_vec_init` is the extension entry point; registering it
        // as an auto-extension is the documented sqlite-vec + rusqlite pattern.
        // Run exactly once via `Once`, before any connection is opened.
        #[allow(clippy::missing_transmute_annotations)]
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
    });
}

/// Type #3 — SQLite connection pool wrapper.
///
/// Rule: `rusqlite::Connection` is `!Send`. This struct never exposes a raw
/// `Connection`. All DB operations use `.interact(|conn| { ... }).await?`
/// so the connection never crosses an await point. See ADR 0004.
#[derive(Debug, Clone)]
pub struct Database {
    pool: Pool,
}

impl Database {
    /// Open (or create) the SQLite database at the given path.
    pub fn open(path: &str) -> Result<Self, MemoryError> {
        // Register sqlite-vec before the pool opens any connection.
        register_sqlite_vec();
        let pool = Config::new(path)
            .create_pool(Runtime::Tokio1)
            .map_err(|e| MemoryError::MigrationFailed(e.to_string()))?;
        Ok(Self { pool })
    }

    /// Expose pool for raw access in tests and CLI commands.
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Run all pending migrations. Safe to call on every startup.
    pub async fn init_schema(&self) -> Result<(), MemoryError> {
        self.pool
            .get()
            .await?
            .interact(|conn| migrations::run(conn))
            .await?
    }

    // ── Write operations ──────────────────────────────────────────────────────

    /// Ensure a session row exists; inserts if absent (upsert by id).
    pub async fn ensure_session(&self, session_id: &str) -> Result<(), MemoryError> {
        let id = session_id.to_string();
        self.pool
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<()> {
                conn.execute("INSERT OR IGNORE INTO sessions (id) VALUES (?1)", [&id])?;
                Ok(())
            })
            .await??;
        Ok(())
    }

    /// Persist a single message. Auto-creates the session row if missing.
    pub async fn save_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
    ) -> Result<(), MemoryError> {
        let sid = session_id.to_string();
        let role = role.to_string();
        let content = content.to_string();
        self.pool
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<()> {
                conn.execute("INSERT OR IGNORE INTO sessions (id) VALUES (?1)", [&sid])?;
                conn.execute(
                    "INSERT INTO messages (session_id, role, content) VALUES (?1, ?2, ?3)",
                    params![sid, role, content],
                )?;
                Ok(())
            })
            .await??;
        Ok(())
    }

    /// Persist a tool call result.
    pub async fn save_tool_call(
        &self,
        session_id: &str,
        call_id: &str,
        tool_name: &str,
        args: &str,
        result: Option<&str>,
        is_error: bool,
    ) -> Result<(), MemoryError> {
        let sid = session_id.to_string();
        let cid = call_id.to_string();
        let name = tool_name.to_string();
        let args = args.to_string();
        let result = result.map(str::to_string);
        self.pool
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<()> {
                conn.execute(
                    "INSERT INTO tool_calls (session_id, call_id, tool_name, args, result, is_error)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![sid, cid, name, args, result, is_error as i32],
                )?;
                Ok(())
            })
            .await??;
        Ok(())
    }

    // ── Read operations ───────────────────────────────────────────────────────

    /// Return the N most recent messages for a session, oldest-first.
    pub async fn recent_messages(
        &self,
        session_id: &str,
        n: usize,
    ) -> Result<Vec<StoredMessage>, MemoryError> {
        let sid = session_id.to_string();
        let n = n as i64;
        let rows: Vec<StoredMessage> = self
            .pool
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<Vec<StoredMessage>> {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, role, content, created_at
                     FROM messages
                     WHERE session_id = ?1
                     ORDER BY id DESC
                     LIMIT ?2",
                )?;
                stmt.query_map(params![sid, n], StoredMessage::from_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await??;
        // Reverse so oldest-first.
        Ok(rows.into_iter().rev().collect())
    }

    /// Full-text search across all messages. Returns results ranked by BM25.
    pub async fn search_messages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, MemoryError> {
        let query = query.to_string();
        let limit = limit as i64;
        let rows: Vec<StoredMessage> = self
            .pool
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<Vec<StoredMessage>> {
                let mut stmt = conn.prepare(
                    "SELECT m.id, m.session_id, m.role, m.content, m.created_at
                     FROM messages m
                     JOIN messages_fts f ON f.rowid = m.id
                     WHERE messages_fts MATCH ?1
                     ORDER BY rank
                     LIMIT ?2",
                )?;
                stmt.query_map(params![query, limit], StoredMessage::from_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await??;
        Ok(rows)
    }

    // ── Maintenance operations ────────────────────────────────────────────────

    /// Run VACUUM to reclaim disk space and reset WAL.
    pub async fn vacuum(&self) -> Result<(), MemoryError> {
        self.pool
            .get()
            .await?
            .interact(|conn| conn.execute_batch("VACUUM"))
            .await??;
        Ok(())
    }

    /// Return basic stats about the database.
    pub async fn stats(&self) -> Result<DbStats, MemoryError> {
        let s: DbStats = self
            .pool
            .get()
            .await?
            .interact(|conn| -> rusqlite::Result<DbStats> {
                let session_count: i64 =
                    conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?;
                let message_count: i64 =
                    conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))?;
                let page_count: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
                let page_size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
                Ok(DbStats {
                    session_count,
                    message_count,
                    size_bytes: page_count * page_size,
                })
            })
            .await??;
        Ok(s)
    }
}

// ── Value types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub id: i64,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub created_at: i64,
}

impl StoredMessage {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            session_id: row.get(1)?,
            role: row.get(2)?,
            content: row.get(3)?,
            created_at: row.get(4)?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct DbStats {
    pub session_count: i64,
    pub message_count: i64,
    pub size_bytes: i64,
}

// ── Legacy error kept until all callers migrate to MemoryError ────────────────
// TODO(phase2): remove DatabaseError after talon-core migrates.
#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("failed to create connection pool: {0}")]
    Pool(String),
    #[error("query failed: {0}")]
    Query(String),
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    async fn open_db() -> Database {
        let db = Database::open(":memory:").expect("open");
        db.init_schema().await.expect("schema");
        db
    }

    #[test]
    fn database_open_in_memory_succeeds() {
        assert!(Database::open(":memory:").is_ok());
    }

    /// Spike (task 2.5.1): the statically-registered sqlite-vec extension is
    /// available on every pooled connection — `vec_version()` resolves.
    #[tokio::test]
    async fn sqlite_vec_extension_loads_through_pool() {
        let db = Database::open(":memory:").expect("open");
        let version: String = db
            .pool()
            .get()
            .await
            .expect("pool")
            .interact(|conn| conn.query_row("SELECT vec_version()", [], |r| r.get::<_, String>(0)))
            .await
            .expect("interact")
            .expect("vec_version");
        assert!(!version.is_empty(), "vec_version returned empty");
    }

    /// vec0 virtual tables can be created and KNN-queried via the pool.
    #[tokio::test]
    async fn vec0_table_roundtrips_knn() {
        use zerocopy::IntoBytes;
        let db = Database::open(":memory:").expect("open");
        db.pool()
            .get()
            .await
            .expect("pool")
            .interact(|conn| -> rusqlite::Result<()> {
                conn.execute_batch("CREATE VIRTUAL TABLE vt USING vec0(embedding float[3]);")?;
                let a: Vec<f32> = vec![1.0, 0.0, 0.0];
                let b: Vec<f32> = vec![0.0, 1.0, 0.0];
                conn.execute(
                    "INSERT INTO vt(rowid, embedding) VALUES (1, ?1)",
                    [a.as_bytes()],
                )?;
                conn.execute(
                    "INSERT INTO vt(rowid, embedding) VALUES (2, ?1)",
                    [b.as_bytes()],
                )?;
                let query: Vec<f32> = vec![0.9, 0.1, 0.0];
                let nearest: i64 = conn.query_row(
                    "SELECT rowid FROM vt WHERE embedding MATCH ?1 ORDER BY distance LIMIT 1",
                    [query.as_bytes()],
                    |r| r.get(0),
                )?;
                assert_eq!(nearest, 1, "nearest to [0.9,0.1,0] should be row 1");
                Ok(())
            })
            .await
            .expect("interact")
            .expect("vec0 roundtrip");
    }

    #[test]
    fn database_pool_is_accessible() {
        let db = Database::open(":memory:").expect("open");
        let _pool = db.pool();
    }

    #[tokio::test]
    async fn init_schema_creates_tables() {
        open_db().await;
    }

    #[tokio::test]
    async fn init_schema_is_idempotent() {
        let db = Database::open(":memory:").expect("open");
        db.init_schema().await.expect("first");
        db.init_schema().await.expect("second — idempotent");
    }

    #[tokio::test]
    async fn save_message_persists_row() {
        let db = open_db().await;
        db.save_message("s1", "user", "hello").await.expect("save");

        let rows = db.recent_messages("s1", 10).await.expect("recent");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].content, "hello");
        assert_eq!(rows[0].role, "user");
    }

    #[tokio::test]
    async fn save_message_auto_creates_session() {
        let db = open_db().await;
        // No explicit ensure_session call.
        db.save_message("auto-session", "user", "hi")
            .await
            .expect("save");
        let rows = db.recent_messages("auto-session", 1).await.expect("recent");
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn recent_messages_returns_oldest_first() {
        let db = open_db().await;
        db.save_message("s2", "user", "first").await.expect("1");
        db.save_message("s2", "assistant", "second")
            .await
            .expect("2");
        db.save_message("s2", "user", "third").await.expect("3");

        let rows = db.recent_messages("s2", 10).await.expect("recent");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].content, "first");
        assert_eq!(rows[2].content, "third");
    }

    #[tokio::test]
    async fn recent_messages_respects_limit() {
        let db = open_db().await;
        for i in 0..5 {
            db.save_message("s3", "user", &format!("msg{i}"))
                .await
                .expect("save");
        }
        let rows = db.recent_messages("s3", 3).await.expect("recent");
        assert_eq!(rows.len(), 3);
        // Should be the 3 most recent (reversed back to oldest-first).
        assert_eq!(rows[0].content, "msg2");
        assert_eq!(rows[2].content, "msg4");
    }

    #[tokio::test]
    async fn search_messages_fts_returns_match() {
        let db = open_db().await;
        db.save_message("s4", "user", "the quick brown fox")
            .await
            .expect("save");
        db.save_message("s4", "assistant", "hello world")
            .await
            .expect("save");

        let results = db.search_messages("fox", 10).await.expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "the quick brown fox");
    }

    #[tokio::test]
    async fn search_messages_returns_empty_on_no_match() {
        let db = open_db().await;
        db.save_message("s5", "user", "nothing relevant here")
            .await
            .expect("save");

        let results = db.search_messages("xyzzy", 10).await.expect("search");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn save_tool_call_persists_row() {
        let db = open_db().await;
        db.save_message("s6", "user", "run").await.expect("session");
        db.save_tool_call("s6", "tc1", "echo", r#"{"msg":"hi"}"#, Some("hi"), false)
            .await
            .expect("save_tool_call");

        let count: i64 = db
            .pool()
            .get()
            .await
            .expect("pool")
            .interact(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM tool_calls WHERE session_id='s6'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .expect("interact")
            .expect("count");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn stats_returns_correct_counts() {
        let db = open_db().await;
        db.save_message("s7", "user", "a").await.expect("1");
        db.save_message("s7", "user", "b").await.expect("2");

        let s = db.stats().await.expect("stats");
        assert_eq!(s.session_count, 1);
        assert_eq!(s.message_count, 2);
        assert!(s.size_bytes > 0);
    }

    #[tokio::test]
    async fn vacuum_does_not_error() {
        let db = open_db().await;
        db.vacuum().await.expect("vacuum");
    }

    // Keep legacy error test to avoid breaking existing talon-core tests.
    #[test]
    fn database_error_pool_display() {
        let err = DatabaseError::Pool("bad path".to_string());
        assert!(err.to_string().contains("bad path"));
    }
}
