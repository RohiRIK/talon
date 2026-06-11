//! Named API tokens with roles (migration v7, criteria 4–6).
//!
//! The raw token exists only at creation time (returned once, never stored);
//! verification hashes the presented token and looks up the hex digest.
//! Revocation is a tombstone so the audit trail keeps the name↔fingerprint
//! mapping.
//!
//! Connection rule (ADR 0004): every query runs inside
//! `pool.get().await?.interact(|conn| …)`.

use std::str::FromStr;
use std::sync::Arc;

use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{Database, error::MemoryError};

/// What a token may do. `Viewer` = read-only (GET + SSE); `Admin` = full.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenRole {
    Admin,
    Viewer,
}

impl TokenRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            TokenRole::Admin => "admin",
            TokenRole::Viewer => "viewer",
        }
    }
}

impl FromStr for TokenRole {
    type Err = MemoryError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "admin" => Ok(TokenRole::Admin),
            "viewer" => Ok(TokenRole::Viewer),
            other => Err(MemoryError::Cron(format!("unknown token role: {other}"))),
        }
    }
}

/// Listing metadata — everything except the hash (and of course the token).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TokenMeta {
    pub name: String,
    pub role: TokenRole,
    pub created_at: String,
    pub last_used: Option<String>,
    pub revoked: bool,
}

/// SHA-256 hex of a raw token. Public so the audit log can fingerprint with
/// the same digest (first 8 hex chars of this value).
pub fn hash_token(raw: &str) -> String {
    let digest = Sha256::digest(raw.as_bytes());
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn now_utc() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[derive(Clone)]
pub struct TokenStore {
    db: Arc<Database>,
}

impl TokenStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Create a token; returns the raw value — the only time it ever exists
    /// outside a hash. Duplicate names error.
    pub async fn create(&self, name: &str, role: TokenRole) -> Result<String, MemoryError> {
        let raw = format!(
            "talon_{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        );
        let hash = hash_token(&raw);
        let id = Uuid::new_v4().to_string();
        let name = name.to_string();
        let created_at = now_utc();
        let role = role.as_str();

        self.interact(move |conn| {
            conn.execute(
                "INSERT INTO api_tokens (id, name, token_hash, role, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, name, hash, role, created_at],
            )
            .map(|_| ())
        })
        .await?;
        Ok(raw)
    }

    /// Verify a presented token: active row with a matching hash → its
    /// (name, role), updating `last_used`. Revoked/unknown → `None`.
    pub async fn verify(&self, raw: &str) -> Result<Option<(String, TokenRole)>, MemoryError> {
        let hash = hash_token(raw);
        let now = now_utc();
        self.interact(move |conn| {
            let found: Option<(String, String)> = conn
                .query_row(
                    "SELECT name, role FROM api_tokens
                      WHERE token_hash = ?1 AND revoked_at IS NULL",
                    params![hash],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((name, role)) = found {
                conn.execute(
                    "UPDATE api_tokens SET last_used = ?1 WHERE name = ?2",
                    params![now, name],
                )?;
                let role = TokenRole::from_str(&role)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;
                Ok(Some((name, role)))
            } else {
                Ok(None)
            }
        })
        .await
    }

    /// All tokens (including revoked tombstones), newest first. Never exposes
    /// hashes.
    pub async fn list(&self) -> Result<Vec<TokenMeta>, MemoryError> {
        self.interact(|conn| {
            let mut stmt = conn.prepare(
                "SELECT name, role, created_at, last_used, revoked_at
                   FROM api_tokens ORDER BY created_at DESC, name",
            )?;
            let rows = stmt.query_map([], |row| {
                let role: String = row.get(1)?;
                Ok(TokenMeta {
                    name: row.get(0)?,
                    role: TokenRole::from_str(&role)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    created_at: row.get(2)?,
                    last_used: row.get(3)?,
                    revoked: row.get::<_, Option<String>>(4)?.is_some(),
                })
            })?;
            rows.collect()
        })
        .await
    }

    /// Tombstone a token by name; `Ok(true)` when an active token was revoked.
    pub async fn revoke(&self, name: &str) -> Result<bool, MemoryError> {
        let name = name.to_string();
        let now = now_utc();
        self.interact(move |conn| {
            conn.execute(
                "UPDATE api_tokens SET revoked_at = ?1
                  WHERE name = ?2 AND revoked_at IS NULL",
                params![now, name],
            )
            .map(|n| n > 0)
        })
        .await
    }

    /// Whether any active (non-revoked) token exists.
    pub async fn has_active(&self) -> Result<bool, MemoryError> {
        self.interact(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM api_tokens WHERE revoked_at IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n > 0)
        })
        .await
    }

    async fn interact<T, F>(&self, f: F) -> Result<T, MemoryError>
    where
        T: Send + 'static,
        F: FnOnce(&mut rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let conn = self.db.pool().get().await?;
        Ok(conn.interact(move |conn| f(conn)).await??)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    async fn store() -> TokenStore {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        TokenStore::new(db)
    }

    #[tokio::test]
    async fn create_then_verify_roundtrip() {
        let s = store().await;
        let raw = s.create("ci", TokenRole::Admin).await.expect("create");
        assert!(raw.starts_with("talon_"));

        let (name, role) = s.verify(&raw).await.expect("verify").expect("found");
        assert_eq!(name, "ci");
        assert_eq!(role, TokenRole::Admin);
    }

    #[tokio::test]
    async fn verify_updates_last_used_and_unknown_is_none() {
        let s = store().await;
        let raw = s.create("t", TokenRole::Viewer).await.expect("create");
        assert!(s.list().await.expect("list")[0].last_used.is_none());

        s.verify(&raw).await.expect("verify").expect("found");
        assert!(s.list().await.expect("list")[0].last_used.is_some());

        assert!(s.verify("talon_nope").await.expect("verify").is_none());
    }

    #[tokio::test]
    async fn revoked_token_no_longer_verifies_but_stays_listed() {
        let s = store().await;
        let raw = s.create("old", TokenRole::Admin).await.expect("create");

        assert!(s.revoke("old").await.expect("revoke"));
        assert!(s.verify(&raw).await.expect("verify").is_none());
        assert!(!s.revoke("old").await.expect("idempotent"));

        let metas = s.list().await.expect("list");
        assert_eq!(metas.len(), 1, "tombstone remains");
        assert!(metas[0].revoked);
    }

    #[tokio::test]
    async fn list_never_contains_raw_or_hash() {
        let s = store().await;
        let raw = s.create("leakcheck", TokenRole::Viewer).await.expect("create");
        let hash = hash_token(&raw);

        let json = serde_json::to_string(&s.list().await.expect("list")).expect("json");
        assert!(!json.contains(&raw));
        assert!(!json.contains(&hash));
    }

    #[tokio::test]
    async fn duplicate_name_errors() {
        let s = store().await;
        s.create("dup", TokenRole::Admin).await.expect("first");
        assert!(s.create("dup", TokenRole::Admin).await.is_err());
    }

    #[tokio::test]
    async fn has_active_tracks_revocation() {
        let s = store().await;
        assert!(!s.has_active().await.expect("empty"));
        s.create("a", TokenRole::Admin).await.expect("create");
        assert!(s.has_active().await.expect("one"));
        s.revoke("a").await.expect("revoke");
        assert!(!s.has_active().await.expect("revoked"));
    }

    #[test]
    fn hash_is_hex_sha256() {
        let h = hash_token("abc");
        assert_eq!(h.len(), 64);
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
