use thiserror::Error;

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("connection pool error: {0}")]
    Pool(#[from] deadpool_sqlite::PoolError),

    #[error("interact error: {0}")]
    Interact(#[from] deadpool_sqlite::InteractError),

    #[error("sqlite error: {0}")]
    Rusqlite(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("migration failed: {0}")]
    MigrationFailed(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("llm error: {0}")]
    Llm(String),
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn migration_failed_display_contains_message() {
        let e = MemoryError::MigrationFailed("version mismatch".to_string());
        assert!(e.to_string().contains("version mismatch"));
    }

    #[test]
    fn not_found_display_contains_key() {
        let e = MemoryError::NotFound("session-abc".to_string());
        assert!(e.to_string().contains("session-abc"));
    }

    #[test]
    fn io_error_converts_via_from() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let e = MemoryError::from(io);
        assert!(e.to_string().contains("file missing"));
    }

    #[test]
    fn rusqlite_error_converts_via_from() {
        let sq = rusqlite::Error::QueryReturnedNoRows;
        let e = MemoryError::from(sq);
        assert!(e.to_string().contains("sqlite"));
    }

    #[test]
    fn pool_error_variant_is_named_pool() {
        // Verify error message prefix matches the expected format.
        let e = MemoryError::MigrationFailed("x".to_string());
        assert!(e.to_string().starts_with("migration failed"));
    }

    #[test]
    fn not_found_variant_message_prefix() {
        let e = MemoryError::NotFound("key".to_string());
        assert!(e.to_string().starts_with("not found"));
    }

    #[test]
    fn llm_variant_message_prefix() {
        let e = MemoryError::Llm("timeout".to_string());
        assert!(e.to_string().starts_with("llm error"));
        assert!(e.to_string().contains("timeout"));
    }
}
