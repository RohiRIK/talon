//! Error type for the secrets subsystem.
//!
//! Error messages may name a secret *reference* (its name/path) but must never
//! contain a secret *value* — these strings flow into run records and logs.

/// All failures the secrets subsystem can produce.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// The textual reference could not be parsed.
    #[error("malformed secret reference `{0}`")]
    MalformedRef(String),

    /// No provider is registered for the reference's scheme.
    #[error("unknown secret provider scheme `{0}`")]
    UnknownScheme(String),

    /// The provider has no secret under this name/path.
    #[error("secret `{name}` not found in provider `{scheme}`")]
    NotFound { scheme: String, name: String },

    /// The builtin vault's master key is unavailable.
    ///
    /// `hint` names the configured unlock credential(s) — never key material.
    #[error("vault is locked: {hint}")]
    Locked { hint: String },

    /// The provider does not support this operation (e.g. `set` on an
    /// external read-only vault).
    #[error("provider `{scheme}` does not support {op}")]
    Unsupported { scheme: String, op: &'static str },

    /// Encryption/decryption failure (wrong key, corrupt ciphertext).
    #[error("crypto failure: {0}")]
    Crypto(String),

    /// Underlying storage failure (SQLite, keychain, network).
    #[error("storage failure: {0}")]
    Storage(String),
}
