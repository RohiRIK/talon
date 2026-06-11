//! Builtin encrypted vault — `secrets` table in talon.db (migration v6).
//!
//! Envelope encryption (criterion 9): each secret value is sealed
//! AES-256-GCM with its own data key (DEK); the DEK is sealed with the vault
//! master key. The master key never touches the database. Crypto runs outside
//! the pool's `interact` closures; connections never cross an `.await`
//! (ADR 0004).

use std::sync::Arc;
use std::{future::Future, pin::Pin};

use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, KeyInit, OsRng, rand_core::RngCore},
};
use rusqlite::{OptionalExtension, params};
use talon_memory::Database;

use crate::master_key::MasterKey;
use crate::{BUILTIN_SCHEME, SecretError, SecretMeta, SecretProvider, SecretRef, SecretValue};

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

/// One stored row, fetched inside `interact`, decrypted outside.
struct SecretRow {
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
    wrapped_dek: Vec<u8>,
    dek_nonce: Vec<u8>,
}

pub struct BuiltinVault {
    db: Arc<Database>,
    master: MasterKey,
}

impl BuiltinVault {
    pub fn new(db: Arc<Database>, master: MasterKey) -> Self {
        Self { db, master }
    }

    /// Store (or replace) a secret. Only ciphertext reaches SQLite.
    pub async fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        let mut dek = [0u8; KEY_LEN];
        OsRng.fill_bytes(&mut dek);

        let (ciphertext, nonce) = seal(&dek, value.as_bytes())?;
        let (wrapped_dek, dek_nonce) = seal(self.master.bytes(), &dek)?;
        dek.fill(0);

        let name = name.to_string();
        let created_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        self.interact(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO secrets
                     (name, ciphertext, nonce, wrapped_dek, dek_nonce, provider, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'builtin', ?6)",
                params![name, ciphertext, nonce, wrapped_dek, dek_nonce, created_at],
            )
            .map(|_| ())
        })
        .await
    }

    /// Remove a secret; `Ok(true)` when a row was deleted.
    pub async fn delete(&self, name: &str) -> Result<bool, SecretError> {
        let name = name.to_string();
        self.interact(move |conn| {
            conn.execute("DELETE FROM secrets WHERE name = ?1", params![name])
                .map(|n| n > 0)
        })
        .await
    }

    async fn fetch(&self, name: &str) -> Result<Option<SecretRow>, SecretError> {
        let name = name.to_string();
        self.interact(move |conn| {
            conn.query_row(
                "SELECT ciphertext, nonce, wrapped_dek, dek_nonce
                   FROM secrets WHERE name = ?1",
                params![name],
                |row| {
                    Ok(SecretRow {
                        ciphertext: row.get(0)?,
                        nonce: row.get(1)?,
                        wrapped_dek: row.get(2)?,
                        dek_nonce: row.get(3)?,
                    })
                },
            )
            .optional()
        })
        .await
    }

    /// Run a closure on a pooled connection, mapping pool/SQLite failures.
    /// The closure must not capture secrets-relevant plaintext.
    async fn interact<T, F>(&self, f: F) -> Result<T, SecretError>
    where
        T: Send + 'static,
        F: FnOnce(&mut rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let conn = self
            .db
            .pool()
            .get()
            .await
            .map_err(|e| SecretError::Storage(format!("pool: {e}")))?;
        conn.interact(move |conn| f(conn))
            .await
            .map_err(|e| SecretError::Storage(format!("interact: {e}")))?
            .map_err(|e| SecretError::Storage(format!("sqlite: {e}")))
    }
}

impl SecretProvider for BuiltinVault {
    fn scheme(&self) -> &'static str {
        BUILTIN_SCHEME
    }

    fn get<'a>(
        &'a self,
        sref: &'a SecretRef,
    ) -> Pin<Box<dyn Future<Output = Result<SecretValue, SecretError>> + Send + 'a>> {
        Box::pin(async move {
            let row = self
                .fetch(&sref.path)
                .await?
                .ok_or_else(|| SecretError::NotFound {
                    scheme: BUILTIN_SCHEME.to_string(),
                    name: sref.display_name().to_string(),
                })?;

            let dek = open(self.master.bytes(), &row.wrapped_dek, &row.dek_nonce)?;
            let dek: [u8; KEY_LEN] = dek.as_slice().try_into().map_err(|_| {
                SecretError::Crypto("stored data key has the wrong length".to_string())
            })?;
            let plaintext = open(&dek, &row.ciphertext, &row.nonce)?;
            String::from_utf8(plaintext)
                .map(SecretValue::new)
                .map_err(|_| SecretError::Crypto("decrypted value is not UTF-8".to_string()))
        })
    }

    fn list(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SecretMeta>, SecretError>> + Send + '_>> {
        Box::pin(async move {
            self.interact(|conn| {
                let mut stmt =
                    conn.prepare("SELECT name, provider, created_at FROM secrets ORDER BY name")?;
                let rows = stmt.query_map([], |row| {
                    Ok(SecretMeta {
                        name: row.get(0)?,
                        scheme: row.get(1)?,
                        created_at: row.get(2)?,
                    })
                })?;
                rows.collect()
            })
            .await
        })
    }
}

/// AES-256-GCM encrypt with a fresh random nonce; returns (ciphertext, nonce).
fn seal(key: &[u8; KEY_LEN], plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), SecretError> {
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|e| SecretError::Crypto(format!("encrypt: {e}")))?;
    Ok((ciphertext, nonce.to_vec()))
}

fn open(key: &[u8; KEY_LEN], ciphertext: &[u8], nonce: &[u8]) -> Result<Vec<u8>, SecretError> {
    if nonce.len() != NONCE_LEN {
        return Err(SecretError::Crypto(
            "stored nonce has the wrong length".to_string(),
        ));
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| {
            SecretError::Crypto("decrypt failed — wrong master key or corrupt data".to_string())
        })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::master_key::MasterKey;

    async fn vault_with_key(key_byte: u8) -> (Arc<Database>, BuiltinVault) {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        let vault = BuiltinVault::new(Arc::clone(&db), MasterKey::from_bytes([key_byte; 32]));
        (db, vault)
    }

    #[tokio::test]
    async fn round_trip_set_get() {
        let (_db, vault) = vault_with_key(1).await;
        vault
            .set("STRIPE_KEY", "sk_live_abc123")
            .await
            .expect("set");

        let sref = SecretRef::parse("{{secret:STRIPE_KEY}}").expect("ref");
        let value = vault.get(&sref).await.expect("get");
        assert_eq!(value.expose(), "sk_live_abc123");
    }

    #[tokio::test]
    async fn raw_db_contains_no_plaintext() {
        let (db, vault) = vault_with_key(2).await;
        let secret_value = "super-plaintext-value-42";
        vault.set("X", secret_value).await.expect("set");

        // Inspect every BLOB/TEXT column of the row directly (criterion 9).
        let needle = secret_value.as_bytes().to_vec();
        let conn = db.pool().get().await.expect("conn");
        let leaked = conn
            .interact(move |conn| {
                conn.query_row(
                    "SELECT ciphertext, nonce, wrapped_dek, dek_nonce FROM secrets WHERE name='X'",
                    [],
                    |row| {
                        let blobs: [Vec<u8>; 4] =
                            [row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?];
                        Ok(blobs
                            .iter()
                            .any(|b| b.windows(needle.len()).any(|w| w == needle.as_slice())))
                    },
                )
            })
            .await
            .expect("interact")
            .expect("query");
        assert!(!leaked, "plaintext found in stored row");
    }

    #[tokio::test]
    async fn wrong_master_key_is_crypto_error() {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        let vault_a = BuiltinVault::new(Arc::clone(&db), MasterKey::from_bytes([3; 32]));
        vault_a.set("K", "v").await.expect("set");

        let vault_b = BuiltinVault::new(Arc::clone(&db), MasterKey::from_bytes([4; 32]));
        let sref = SecretRef::parse("{{secret:K}}").expect("ref");
        let err = vault_b.get(&sref).await.expect_err("wrong key");
        assert!(matches!(err, SecretError::Crypto(_)));
        assert!(!err.to_string().contains('v'), "error must not leak value");
    }

    #[tokio::test]
    async fn missing_secret_is_not_found_naming_ref() {
        let (_db, vault) = vault_with_key(5).await;
        let sref = SecretRef::parse("{{secret:NOPE}}").expect("ref");
        let err = vault.get(&sref).await.expect_err("missing");
        assert!(err.to_string().contains("NOPE"));
    }

    #[tokio::test]
    async fn list_returns_metadata_only_and_delete_removes() {
        let (_db, vault) = vault_with_key(6).await;
        vault.set("B", "vb").await.expect("set");
        vault.set("A", "va").await.expect("set");

        let metas = vault.list().await.expect("list");
        assert_eq!(metas.len(), 2);
        assert_eq!(metas[0].name, "A"); // ordered
        assert!(metas.iter().all(|m| m.scheme == "builtin"));
        assert!(metas.iter().all(|m| m.created_at.is_some()));

        assert!(vault.delete("A").await.expect("delete"));
        assert!(!vault.delete("A").await.expect("idempotent"));
        assert_eq!(vault.list().await.expect("list").len(), 1);
    }

    #[tokio::test]
    async fn set_overwrites_existing() {
        let (_db, vault) = vault_with_key(7).await;
        vault.set("K", "old").await.expect("set");
        vault.set("K", "new").await.expect("overwrite");

        let sref = SecretRef::parse("{{secret:K}}").expect("ref");
        assert_eq!(vault.get(&sref).await.expect("get").expose(), "new");
    }
}
