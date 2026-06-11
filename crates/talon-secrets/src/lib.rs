//! talon-secrets — secret references, providers, and the builtin encrypted vault.
//!
//! Home of the `SecretProvider` trait (defined ONCE here, load-bearing-types
//! discipline). Values resolved through this crate are registered for
//! redaction and must never reach a persistence or logging sink in plaintext.

mod builtin;
mod env;
mod error;
mod master_key;
mod secret_ref;

pub use builtin::BuiltinVault;
pub use env::{ENV_SCHEME, EnvProvider};
pub use error::SecretError;
pub use master_key::{
    Credential, ENV_VAR, KEYCHAIN_ENTRY, KEYCHAIN_SERVICE, KeychainStore, MasterKey,
    MasterKeyStore, OsKeychain, RECOVERY_FILE,
};
pub use secret_ref::{BUILTIN_SCHEME, SecretRef};

use std::{future::Future, pin::Pin};

/// A resolved secret value.
///
/// `Debug`/`Display` never print the contained value; call
/// [`SecretValue::expose`] at the single point of use.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Deliberately loud accessor — grep for `.expose()` to audit value flow.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue([REDACTED])")
    }
}

/// Listing metadata — everything about a secret except its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretMeta {
    pub name: String,
    pub scheme: String,
    pub created_at: Option<String>,
}

/// The provider interface. Read-only by design: `set`/`delete` exist only as
/// inherent methods on providers that support them (the builtin vault).
///
/// Methods return `Pin<Box<dyn Future>>` rather than `async fn` so the trait
/// is dyn-compatible — required by `Arc<dyn SecretProvider>` (same pattern as
/// `Tool`/`LlmProvider`, ADR 0007).
pub trait SecretProvider: Send + Sync {
    /// The URI scheme this provider answers for (`builtin`, `env`, …).
    fn scheme(&self) -> &'static str;

    /// Resolve one reference to its value.
    fn get<'a>(
        &'a self,
        sref: &'a SecretRef,
    ) -> Pin<Box<dyn Future<Output = Result<SecretValue, SecretError>> + Send + 'a>>;

    /// List metadata for everything this provider holds. Providers that
    /// cannot enumerate (e.g. env) return an empty list.
    fn list(&self) -> Pin<Box<dyn Future<Output = Result<Vec<SecretMeta>, SecretError>> + Send + '_>>;
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct StaticProvider;

    impl SecretProvider for StaticProvider {
        fn scheme(&self) -> &'static str {
            "static"
        }

        fn get<'a>(
            &'a self,
            sref: &'a SecretRef,
        ) -> Pin<Box<dyn Future<Output = Result<SecretValue, SecretError>> + Send + 'a>> {
            Box::pin(async move {
                if sref.path == "known" {
                    Ok(SecretValue::new("s3cr3t"))
                } else {
                    Err(SecretError::NotFound {
                        scheme: self.scheme().to_string(),
                        name: sref.display_name().to_string(),
                    })
                }
            })
        }

        fn list(
            &self,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<SecretMeta>, SecretError>> + Send + '_>>
        {
            Box::pin(async { Ok(vec![]) })
        }
    }

    /// `Arc<dyn SecretProvider>` must be usable — dyn-compatibility check
    /// (criterion 12).
    #[tokio::test]
    async fn arc_dyn_provider_resolves() {
        let provider: Arc<dyn SecretProvider> = Arc::new(StaticProvider);
        let sref = SecretRef::parse("secret://static/known").expect("valid");
        let value = provider.get(&sref).await.expect("resolves");
        assert_eq!(value.expose(), "s3cr3t");
    }

    #[tokio::test]
    async fn not_found_error_names_ref_not_value() {
        let provider: Arc<dyn SecretProvider> = Arc::new(StaticProvider);
        let sref = SecretRef::parse("secret://static/missing").expect("valid");
        let err = provider.get(&sref).await.expect_err("missing");
        let msg = err.to_string();
        assert!(msg.contains("secret://static/missing"));
        assert!(!msg.contains("s3cr3t"));
    }

    #[test]
    fn secret_value_debug_is_redacted() {
        let v = SecretValue::new("hunter2");
        assert_eq!(format!("{v:?}"), "SecretValue([REDACTED])");
        assert!(!format!("{v:?}").contains("hunter2"));
    }
}
