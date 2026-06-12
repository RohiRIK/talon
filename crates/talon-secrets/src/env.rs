//! `EnvProvider` — `secret://env/FOO` resolves from the process environment.
//!
//! Always available (criterion 12): the universal escape hatch for CI and
//! containers. Cannot enumerate (no trustworthy way to tell secrets from
//! ordinary env vars), so `list` returns empty.

use std::{future::Future, pin::Pin};

use crate::{SecretError, SecretMeta, SecretProvider, SecretRef, SecretValue};

pub const ENV_SCHEME: &str = "env";

pub struct EnvProvider;

impl SecretProvider for EnvProvider {
    fn scheme(&self) -> &'static str {
        ENV_SCHEME
    }

    fn get<'a>(
        &'a self,
        sref: &'a SecretRef,
    ) -> Pin<Box<dyn Future<Output = Result<SecretValue, SecretError>> + Send + 'a>> {
        Box::pin(async move {
            match std::env::var(&sref.path) {
                Ok(value) => Ok(SecretValue::new(value)),
                Err(_) => Err(SecretError::NotFound {
                    scheme: ENV_SCHEME.to_string(),
                    name: sref.display_name().to_string(),
                }),
            }
        })
    }

    fn list(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SecretMeta>, SecretError>> + Send + '_>> {
        Box::pin(async { Ok(vec![]) })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolves_set_env_var() {
        // Process-global env: use a name no other test touches.
        unsafe { std::env::set_var("TALON_SECRETS_TEST_FOO", "from-env") };
        let sref = SecretRef::parse("secret://env/TALON_SECRETS_TEST_FOO").expect("ref");
        let value = EnvProvider.get(&sref).await.expect("resolves");
        assert_eq!(value.expose(), "from-env");
    }

    #[tokio::test]
    async fn unset_var_is_not_found() {
        let sref = SecretRef::parse("secret://env/TALON_SECRETS_TEST_UNSET").expect("ref");
        let err = EnvProvider.get(&sref).await.expect_err("unset");
        assert!(matches!(err, SecretError::NotFound { .. }));
    }

    #[tokio::test]
    async fn list_is_empty() {
        assert!(EnvProvider.list().await.expect("list").is_empty());
    }
}
