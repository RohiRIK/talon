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
}
