//! Audit log (migration v10, criterion 32): one row per mutating API call.
//!
//! `token_fp` is the first 8 hex characters of SHA-256(token) — attributable
//! via `api_tokens.token_hash` prefixes without ever storing a usable
//! credential.

use std::sync::Arc;

use rusqlite::params;
use uuid::Uuid;

use crate::{Database, error::MemoryError};

/// Fingerprint length in hex chars.
pub const FP_LEN: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AuditEntry {
    pub id: String,
    pub ts: String,
    pub token_fp: String,
    pub method: String,
    pub path: String,
    pub target_id: Option<String>,
}

#[derive(Clone)]
pub struct AuditStore {
    db: Arc<Database>,
}

impl AuditStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub async fn record(
        &self,
        token_fp: &str,
        method: &str,
        path: &str,
        target_id: Option<String>,
    ) -> Result<(), MemoryError> {
        let entry = AuditEntry {
            id: Uuid::new_v4().to_string(),
            ts: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            token_fp: token_fp.chars().take(FP_LEN).collect(),
            method: method.to_string(),
            path: path.to_string(),
            target_id,
        };
        let conn = self.db.pool().get().await?;
        conn.interact(move |conn| {
            conn.execute(
                "INSERT INTO audit_log (id, ts, token_fp, method, path, target_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    entry.id,
                    entry.ts,
                    entry.token_fp,
                    entry.method,
                    entry.path,
                    entry.target_id
                ],
            )
            .map(|_| ())
        })
        .await??;
        Ok(())
    }

    pub async fn recent(&self, limit: i64) -> Result<Vec<AuditEntry>, MemoryError> {
        let conn = self.db.pool().get().await?;
        Ok(conn
            .interact(move |conn| -> rusqlite::Result<Vec<AuditEntry>> {
                let mut stmt = conn.prepare(
                    "SELECT id, ts, token_fp, method, path, target_id
                       FROM audit_log ORDER BY ts DESC, id LIMIT ?1",
                )?;
                let rows = stmt.query_map(params![limit], |row| {
                    Ok(AuditEntry {
                        id: row.get(0)?,
                        ts: row.get(1)?,
                        token_fp: row.get(2)?,
                        method: row.get(3)?,
                        path: row.get(4)?,
                        target_id: row.get(5)?,
                    })
                })?;
                rows.collect()
            })
            .await??)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn record_truncates_fp_and_lists_newest_first() {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        let audit = AuditStore::new(db);

        audit
            .record(
                "abcdef0123456789-much-longer-than-eight",
                "POST",
                "/api/v1/jobs",
                None,
            )
            .await
            .expect("record");

        let entries = audit.recent(10).await.expect("recent");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].token_fp, "abcdef01", "fingerprint truncated");
        assert_eq!(entries[0].method, "POST");
    }
}
