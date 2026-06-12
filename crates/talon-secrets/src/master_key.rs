//! Master-key lifecycle: authenticated bootstrap, unlock chain, rewrap.
//!
//! Spec criteria 1–3. The master key is generated ONLY after an unlock
//! credential is established, and is persisted only in wrapped form:
//!
//! - OS keychain entry `talon-master-key` (base64 of the raw 32 bytes —
//!   access gated by the OS user session), and/or
//! - argon2id-wrapped recovery blob `~/.talon/master.key.enc`.
//!
//! Unlock chain: keychain → `TALON_MASTER_KEY` env → passphrase against the
//! recovery blob → `SecretError::Locked`. A locked vault never generates a
//! replacement key — that would orphan existing ciphertext.

use std::path::{Path, PathBuf};

use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, KeyInit, OsRng, rand_core::RngCore},
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;

use crate::SecretError;

/// Keychain entry name — distinct from the per-provider LLM keys
/// (`<provider>-api-key`) managed by the init wizard.
pub const KEYCHAIN_ENTRY: &str = "talon-master-key";
/// Keychain service, matching the existing wizard convention.
pub const KEYCHAIN_SERVICE: &str = "talon";
/// Recovery blob filename inside the talon home directory.
pub const RECOVERY_FILE: &str = "master.key.enc";
/// Env var carrying the base64 raw key (headless/CI path).
pub const ENV_VAR: &str = "TALON_MASTER_KEY";

const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
/// Recovery blob magic + format version.
const MAGIC: &[u8; 8] = b"TALONMK1";

/// The raw vault master key. Never serialized; `Debug` is redacted; zeroed on
/// drop (best-effort).
pub struct MasterKey([u8; KEY_LEN]);

impl MasterKey {
    /// Construct from raw key bytes. Intended for tests and embedders that
    /// manage key material themselves — production flows go through
    /// [`MasterKeyStore::bootstrap`]/[`MasterKeyStore::unlock`].
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    pub(crate) fn bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey([REDACTED])")
    }
}

impl Drop for MasterKey {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

/// Keychain access behind a trait so tests never touch the real OS keychain.
pub trait KeychainStore: Send + Sync {
    /// Returns the stored raw key bytes, or `None` when no entry exists.
    fn get(&self) -> Result<Option<Vec<u8>>, SecretError>;
    fn set(&self, key: &[u8]) -> Result<(), SecretError>;
    fn delete(&self) -> Result<(), SecretError>;
    /// Whether a keychain is usable on this machine at all.
    fn available(&self) -> bool;
}

/// Real OS keychain via the `keyring` crate (same crate the init wizard uses).
pub struct OsKeychain;

impl KeychainStore for OsKeychain {
    fn get(&self) -> Result<Option<Vec<u8>>, SecretError> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ENTRY)
            .map_err(|e| SecretError::Storage(format!("keychain: {e}")))?;
        match entry.get_password() {
            Ok(b64) => {
                let bytes = B64
                    .decode(b64.trim())
                    .map_err(|e| SecretError::Crypto(format!("keychain entry corrupt: {e}")))?;
                Ok(Some(bytes))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError::Storage(format!("keychain: {e}"))),
        }
    }

    fn set(&self, key: &[u8]) -> Result<(), SecretError> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ENTRY)
            .map_err(|e| SecretError::Storage(format!("keychain: {e}")))?;
        entry
            .set_password(&B64.encode(key))
            .map_err(|e| SecretError::Storage(format!("keychain: {e}")))
    }

    fn delete(&self) -> Result<(), SecretError> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ENTRY)
            .map_err(|e| SecretError::Storage(format!("keychain: {e}")))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError::Storage(format!("keychain: {e}"))),
        }
    }

    fn available(&self) -> bool {
        true
    }
}

/// An unlock credential chosen at `talon init`.
pub enum Credential {
    /// Wrap in the OS keychain.
    Keychain,
    /// Wrap in an argon2id recovery blob with this passphrase.
    Passphrase(String),
}

/// Manages wrapped copies of the master key for one talon home directory.
pub struct MasterKeyStore<'a> {
    talon_home: PathBuf,
    keychain: &'a dyn KeychainStore,
}

impl<'a> MasterKeyStore<'a> {
    pub fn new(talon_home: impl Into<PathBuf>, keychain: &'a dyn KeychainStore) -> Self {
        Self {
            talon_home: talon_home.into(),
            keychain,
        }
    }

    pub fn recovery_blob_path(&self) -> PathBuf {
        self.talon_home.join(RECOVERY_FILE)
    }

    /// True when any wrapped copy exists (bootstrap already happened).
    pub fn is_bootstrapped(&self) -> Result<bool, SecretError> {
        Ok(self.keychain.get()?.is_some() || self.recovery_blob_path().exists())
    }

    /// Authenticated bootstrap (criterion 1): refuse to generate without a
    /// credential; generate only after wraps are validated; on any failure,
    /// roll back every artifact written so far — abort leaves zero key
    /// material behind.
    pub fn bootstrap(&self, credentials: &[Credential]) -> Result<MasterKey, SecretError> {
        if credentials.is_empty() {
            return Err(SecretError::Storage(
                "refusing to generate a master key without an unlock credential — \
                 choose the OS keychain and/or a passphrase first"
                    .to_string(),
            ));
        }
        if self.is_bootstrapped()? {
            return Err(SecretError::Storage(
                "a master key already exists — use `talon secret rewrap` to change wraps"
                    .to_string(),
            ));
        }

        let mut raw = [0u8; KEY_LEN];
        OsRng.fill_bytes(&mut raw);
        let key = MasterKey::from_bytes(raw);

        let mut wrote_keychain = false;
        let mut wrote_blob = false;
        let result = (|| {
            for cred in credentials {
                match cred {
                    Credential::Keychain => {
                        self.keychain.set(key.bytes())?;
                        wrote_keychain = true;
                    }
                    Credential::Passphrase(pass) => {
                        self.write_recovery_blob(&key, pass)?;
                        wrote_blob = true;
                    }
                }
            }
            Ok(())
        })();

        match result {
            Ok(()) => Ok(key),
            Err(e) => {
                // Roll back partial writes — abort must leave nothing behind.
                if wrote_keychain {
                    let _ = self.keychain.delete();
                }
                if wrote_blob {
                    let _ = std::fs::remove_file(self.recovery_blob_path());
                }
                Err(e)
            }
        }
    }

    /// Unlock chain (criterion 2). `env_value` is the raw `TALON_MASTER_KEY`
    /// content (caller reads the environment); `passphrase` is supplied by the
    /// caller's prompt when a TTY exists.
    pub fn unlock(
        &self,
        env_value: Option<&str>,
        passphrase: Option<&str>,
    ) -> Result<MasterKey, SecretError> {
        if let Some(bytes) = self.keychain.get()? {
            return key_from_slice(&bytes, "keychain entry");
        }

        if let Some(b64) = env_value {
            let bytes = B64
                .decode(b64.trim())
                .map_err(|e| SecretError::Crypto(format!("{ENV_VAR} is not valid base64: {e}")))?;
            return key_from_slice(&bytes, ENV_VAR);
        }

        let blob_path = self.recovery_blob_path();
        if blob_path.exists()
            && let Some(pass) = passphrase
        {
            return self.read_recovery_blob(pass);
        }

        Err(SecretError::Locked {
            hint: self.locked_hint(blob_path.exists()),
        })
    }

    /// Add or rotate a passphrase wrap (criterion 3). Only the wrap changes —
    /// per-secret ciphertext is untouched by construction (the master key
    /// itself does not change).
    pub fn rewrap_passphrase(&self, key: &MasterKey, passphrase: &str) -> Result<(), SecretError> {
        self.write_recovery_blob(key, passphrase)
    }

    /// (Re-)add the keychain wrap.
    pub fn rewrap_keychain(&self, key: &MasterKey) -> Result<(), SecretError> {
        self.keychain.set(key.bytes())
    }

    fn locked_hint(&self, blob_exists: bool) -> String {
        let recovery = if blob_exists {
            "a recovery blob exists — unlock with your passphrase"
        } else {
            "no recovery blob found"
        };
        format!(
            "no keychain entry, {ENV_VAR} unset, {recovery}. \
             Unlock with one of those, or run `talon init` on a fresh install."
        )
    }

    /// Blob layout: MAGIC(8) ‖ salt(16) ‖ nonce(12) ‖ ciphertext(32+16 tag).
    fn write_recovery_blob(&self, key: &MasterKey, passphrase: &str) -> Result<(), SecretError> {
        let mut salt = [0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);
        let kek = derive_kek(passphrase, &salt)?;

        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&kek));
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), key.bytes().as_slice())
            .map_err(|e| SecretError::Crypto(format!("recovery blob encrypt: {e}")))?;

        let mut blob = Vec::with_capacity(MAGIC.len() + SALT_LEN + NONCE_LEN + ciphertext.len());
        blob.extend_from_slice(MAGIC);
        blob.extend_from_slice(&salt);
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&ciphertext);

        write_atomic(&self.recovery_blob_path(), &blob)
    }

    fn read_recovery_blob(&self, passphrase: &str) -> Result<MasterKey, SecretError> {
        let blob = std::fs::read(self.recovery_blob_path())
            .map_err(|e| SecretError::Storage(format!("recovery blob: {e}")))?;

        let min = MAGIC.len() + SALT_LEN + NONCE_LEN + KEY_LEN;
        if blob.len() < min || &blob[..MAGIC.len()] != MAGIC {
            return Err(SecretError::Crypto(
                "recovery blob is corrupt or from an unknown version".to_string(),
            ));
        }
        let salt = &blob[MAGIC.len()..MAGIC.len() + SALT_LEN];
        let nonce = &blob[MAGIC.len() + SALT_LEN..MAGIC.len() + SALT_LEN + NONCE_LEN];
        let ciphertext = &blob[MAGIC.len() + SALT_LEN + NONCE_LEN..];

        let kek = derive_kek(passphrase, salt)?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&kek));
        let plaintext = cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| {
                SecretError::Crypto("wrong passphrase or corrupt recovery blob".to_string())
            })?;

        key_from_slice(&plaintext, "recovery blob")
    }
}

fn key_from_slice(bytes: &[u8], source: &str) -> Result<MasterKey, SecretError> {
    let arr: [u8; KEY_LEN] = bytes
        .try_into()
        .map_err(|_| SecretError::Crypto(format!("{source} does not contain a 32-byte key")))?;
    Ok(MasterKey::from_bytes(arr))
}

fn derive_kek(passphrase: &str, salt: &[u8]) -> Result<[u8; KEY_LEN], SecretError> {
    let mut kek = [0u8; KEY_LEN];
    argon2::Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut kek)
        .map_err(|e| SecretError::Crypto(format!("argon2id: {e}")))?;
    Ok(kek)
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), SecretError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| SecretError::Storage(format!("create {}: {e}", parent.display())))?;
    }
    let tmp = path.with_extension("enc.tmp");
    std::fs::write(&tmp, contents)
        .map_err(|e| SecretError::Storage(format!("write {}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| SecretError::Storage(format!("rename to {}: {e}", path.display())))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    /// In-memory keychain double — tests never touch the OS keychain.
    pub(crate) struct MockKeychain {
        entry: Mutex<Option<Vec<u8>>>,
        pub available: bool,
    }

    impl MockKeychain {
        pub(crate) fn new() -> Self {
            Self {
                entry: Mutex::new(None),
                available: true,
            }
        }
    }

    impl KeychainStore for MockKeychain {
        fn get(&self) -> Result<Option<Vec<u8>>, SecretError> {
            Ok(self.entry.lock().expect("lock").clone())
        }
        fn set(&self, key: &[u8]) -> Result<(), SecretError> {
            *self.entry.lock().expect("lock") = Some(key.to_vec());
            Ok(())
        }
        fn delete(&self) -> Result<(), SecretError> {
            *self.entry.lock().expect("lock") = None;
            Ok(())
        }
        fn available(&self) -> bool {
            self.available
        }
    }

    /// A keychain that fails on `set` — exercises bootstrap rollback.
    struct FailingKeychain;

    impl KeychainStore for FailingKeychain {
        fn get(&self) -> Result<Option<Vec<u8>>, SecretError> {
            Ok(None)
        }
        fn set(&self, _key: &[u8]) -> Result<(), SecretError> {
            Err(SecretError::Storage("keychain unavailable".to_string()))
        }
        fn delete(&self) -> Result<(), SecretError> {
            Ok(())
        }
        fn available(&self) -> bool {
            false
        }
    }

    pub(crate) fn temp_home(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("talon-secrets-test-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp home");
        dir
    }

    #[test]
    fn bootstrap_requires_a_credential() {
        let home = temp_home("no-cred");
        let kc = MockKeychain::new();
        let store = MasterKeyStore::new(&home, &kc);

        let err = store.bootstrap(&[]).expect_err("must refuse");
        let msg = err.to_string();
        assert!(msg.contains("credential"), "actionable: {msg}");
        // Zero artifacts.
        assert!(!store.recovery_blob_path().exists());
        assert!(kc.get().expect("get").is_none());
    }

    #[test]
    fn bootstrap_passphrase_writes_only_wrapped_blob_and_unlocks() {
        let home = temp_home("pass");
        let kc = MockKeychain::new();
        let store = MasterKeyStore::new(&home, &kc);

        let key = store
            .bootstrap(&[Credential::Passphrase("correct horse".to_string())])
            .expect("bootstrap");

        let blob = std::fs::read(store.recovery_blob_path()).expect("blob exists");
        // Wrapped only: raw key bytes must not appear in the blob.
        assert!(
            !blob.windows(KEY_LEN).any(|w| w == key.bytes().as_slice()),
            "raw key leaked into recovery blob"
        );

        let unlocked = store
            .unlock(None, Some("correct horse"))
            .expect("unlock via passphrase");
        assert_eq!(unlocked.bytes(), key.bytes());
    }

    #[test]
    fn wrong_passphrase_is_crypto_error_not_panic() {
        let home = temp_home("wrong-pass");
        let kc = MockKeychain::new();
        let store = MasterKeyStore::new(&home, &kc);
        store
            .bootstrap(&[Credential::Passphrase("right".to_string())])
            .expect("bootstrap");

        let err = store.unlock(None, Some("wrong")).expect_err("wrong pass");
        assert!(matches!(err, SecretError::Crypto(_)));
    }

    #[test]
    fn unlock_via_keychain_and_env() {
        let home = temp_home("chain");
        let kc = MockKeychain::new();
        let store = MasterKeyStore::new(&home, &kc);
        let key = store.bootstrap(&[Credential::Keychain]).expect("bootstrap");

        // Keychain path.
        let via_kc = store.unlock(None, None).expect("keychain unlock");
        assert_eq!(via_kc.bytes(), key.bytes());

        // Env path (headless sim): drop the keychain entry, supply base64.
        kc.delete().expect("delete");
        let b64 = B64.encode(key.bytes());
        let via_env = store.unlock(Some(&b64), None).expect("env unlock");
        assert_eq!(via_env.bytes(), key.bytes());
    }

    #[test]
    fn locked_error_names_the_options() {
        let home = temp_home("locked");
        let kc = MockKeychain::new();
        let store = MasterKeyStore::new(&home, &kc);

        let err = store.unlock(None, None).expect_err("locked");
        match err {
            SecretError::Locked { hint } => {
                assert!(hint.contains(ENV_VAR));
                assert!(hint.contains("keychain"));
            }
            other => panic!("expected Locked, got {other:?}"),
        }
    }

    #[test]
    fn bootstrap_rollback_leaves_zero_artifacts() {
        let home = temp_home("rollback");
        let kc = FailingKeychain;
        let store = MasterKeyStore::new(&home, &kc);

        // Blob first, then keychain fails → blob must be rolled back.
        let err = store
            .bootstrap(&[
                Credential::Passphrase("p".to_string()),
                Credential::Keychain,
            ])
            .expect_err("keychain set fails");
        assert!(matches!(err, SecretError::Storage(_)));
        assert!(
            !store.recovery_blob_path().exists(),
            "partial blob must be rolled back"
        );
    }

    #[test]
    fn rewrap_passphrase_survives_keychain_loss() {
        let home = temp_home("rewrap");
        let kc = MockKeychain::new();
        let store = MasterKeyStore::new(&home, &kc);
        let key = store.bootstrap(&[Credential::Keychain]).expect("bootstrap");

        // Add a passphrase wrap later (criterion 3).
        store
            .rewrap_passphrase(&key, "recovery phrase")
            .expect("rewrap");

        // Lose the keychain entry — recovery blob still unlocks.
        kc.delete().expect("delete");
        let recovered = store
            .unlock(None, Some("recovery phrase"))
            .expect("recovery unlock");
        assert_eq!(recovered.bytes(), key.bytes());
    }

    #[test]
    fn double_bootstrap_refused() {
        let home = temp_home("double");
        let kc = MockKeychain::new();
        let store = MasterKeyStore::new(&home, &kc);
        store.bootstrap(&[Credential::Keychain]).expect("first");

        let err = store
            .bootstrap(&[Credential::Keychain])
            .expect_err("second must refuse");
        assert!(err.to_string().contains("rewrap"));
    }

    #[test]
    fn master_key_debug_is_redacted() {
        let key = MasterKey::from_bytes([7u8; KEY_LEN]);
        assert_eq!(format!("{key:?}"), "MasterKey([REDACTED])");
    }
}
