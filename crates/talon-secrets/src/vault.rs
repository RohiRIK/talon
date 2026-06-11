//! HashiCorp Vault provider — KV v2, AppRole auth, read-only (criterion 13).
//!
//! Reference syntax: `secret://vault/<mount>/<path>#<key>` — the first path
//! segment is the KV mount, the rest is the secret path. The `#<key>` picks a
//! field from the secret's data map; without it, a single-field secret
//! resolves to that field and a multi-field secret is an error naming the
//! available keys (names only, never values).
//!
//! AppRole tokens are cached and renewed before expiry; a 403 triggers one
//! re-login retry (token may have been revoked server-side). Write operations
//! do not exist here — rotation is the external vault's job.

use std::time::{Duration, Instant};
use std::{future::Future, pin::Pin};

use tokio::sync::RwLock;

use crate::{SecretError, SecretMeta, SecretProvider, SecretRef, SecretValue};

pub const VAULT_SCHEME: &str = "vault";

/// Renew this long before the token's actual expiry.
const EXPIRY_MARGIN_SECS: u64 = 30;

#[derive(Debug, Clone)]
pub struct VaultConfig {
    /// Base address, e.g. `https://vault.internal:8200`.
    pub addr: String,
    /// AppRole credentials. `secret_id` is read from an env var by the
    /// caller (never from config.toml).
    pub role_id: String,
    pub secret_id: String,
}

struct CachedToken {
    token: String,
    expires_at: Instant,
}

pub struct VaultProvider {
    cfg: VaultConfig,
    http: reqwest::Client,
    cached: RwLock<Option<CachedToken>>,
}

impl VaultProvider {
    pub fn new(cfg: VaultConfig) -> Self {
        Self {
            cfg,
            http: reqwest::Client::new(),
            cached: RwLock::new(None),
        }
    }

    /// Valid cached token, or AppRole login.
    async fn token(&self) -> Result<String, SecretError> {
        if let Some(cached) = self.cached.read().await.as_ref()
            && cached.expires_at > Instant::now()
        {
            return Ok(cached.token.clone());
        }
        self.login().await
    }

    async fn login(&self) -> Result<String, SecretError> {
        let url = format!(
            "{}/v1/auth/approle/login",
            self.cfg.addr.trim_end_matches('/')
        );
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "role_id": self.cfg.role_id,
                "secret_id": self.cfg.secret_id,
            }))
            .send()
            .await
            .map_err(|e| SecretError::Storage(format!("vault login: {e}")))?;

        if !resp.status().is_success() {
            return Err(SecretError::Storage(format!(
                "vault AppRole login failed: HTTP {}",
                resp.status()
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SecretError::Storage(format!("vault login response: {e}")))?;
        let token = body["auth"]["client_token"]
            .as_str()
            .ok_or_else(|| SecretError::Storage("vault login: no client_token".to_string()))?
            .to_string();
        let lease = body["auth"]["lease_duration"].as_u64().unwrap_or(300);

        let expires_at =
            Instant::now() + Duration::from_secs(lease.saturating_sub(EXPIRY_MARGIN_SECS).max(1));
        *self.cached.write().await = Some(CachedToken {
            token: token.clone(),
            expires_at,
        });
        Ok(token)
    }

    /// KV v2 read: `GET /v1/<mount>/data/<path>`.
    async fn read(
        &self,
        token: &str,
        mount: &str,
        path: &str,
    ) -> Result<reqwest::Response, SecretError> {
        let url = format!(
            "{}/v1/{mount}/data/{path}",
            self.cfg.addr.trim_end_matches('/')
        );
        self.http
            .get(&url)
            .header("X-Vault-Token", token)
            .send()
            .await
            .map_err(|e| SecretError::Storage(format!("vault read: {e}")))
    }

    fn split_mount(sref: &SecretRef) -> Result<(&str, &str), SecretError> {
        sref.path
            .split_once('/')
            .filter(|(m, p)| !m.is_empty() && !p.is_empty())
            .ok_or_else(|| SecretError::MalformedRef(sref.raw.clone()))
    }

    fn extract_field(
        sref: &SecretRef,
        data: &serde_json::Value,
    ) -> Result<SecretValue, SecretError> {
        let map = data
            .as_object()
            .ok_or_else(|| SecretError::Storage("vault: data is not an object".to_string()))?;

        let field = match &sref.key {
            Some(key) => map.get(key.as_str()).ok_or_else(|| SecretError::NotFound {
                scheme: VAULT_SCHEME.to_string(),
                name: sref.display_name().to_string(),
            })?,
            None if map.len() == 1 => map
                .values()
                .next()
                .ok_or_else(|| SecretError::Storage("vault: empty data map".to_string()))?,
            None => {
                let keys: Vec<&str> = map.keys().map(String::as_str).collect();
                return Err(SecretError::Storage(format!(
                    "vault secret has multiple fields {keys:?} — pick one with #<key>"
                )));
            }
        };

        match field.as_str() {
            Some(s) => Ok(SecretValue::new(s)),
            None => Ok(SecretValue::new(field.to_string())),
        }
    }
}

impl SecretProvider for VaultProvider {
    fn scheme(&self) -> &'static str {
        VAULT_SCHEME
    }

    fn get<'a>(
        &'a self,
        sref: &'a SecretRef,
    ) -> Pin<Box<dyn Future<Output = Result<SecretValue, SecretError>> + Send + 'a>> {
        Box::pin(async move {
            let (mount, path) = Self::split_mount(sref)?;
            let token = self.token().await?;

            let mut resp = self.read(&token, mount, path).await?;
            // One retry after re-login: cached token may be revoked.
            if resp.status() == reqwest::StatusCode::FORBIDDEN {
                *self.cached.write().await = None;
                let fresh = self.login().await?;
                resp = self.read(&fresh, mount, path).await?;
            }

            match resp.status() {
                s if s.is_success() => {}
                reqwest::StatusCode::NOT_FOUND => {
                    return Err(SecretError::NotFound {
                        scheme: VAULT_SCHEME.to_string(),
                        name: sref.display_name().to_string(),
                    });
                }
                s => {
                    return Err(SecretError::Storage(format!("vault read: HTTP {s}")));
                }
            }

            let body: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| SecretError::Storage(format!("vault read body: {e}")))?;
            Self::extract_field(sref, &body["data"]["data"])
        })
    }

    /// Vault enumeration is policy-dependent and noisy — not supported v1.
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
    use wiremock::matchers::{body_json_string, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn login_ok(token: &str, lease: u64) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "auth": { "client_token": token, "lease_duration": lease }
        }))
    }

    fn kv_ok(data: serde_json::Value) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "data": data }
        }))
    }

    async fn provider(server: &MockServer) -> VaultProvider {
        VaultProvider::new(VaultConfig {
            addr: server.uri(),
            role_id: "rid".to_string(),
            secret_id: "sid".to_string(),
        })
    }

    #[tokio::test]
    async fn login_then_read_with_key_fragment() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/approle/login"))
            .and(body_json_string(
                serde_json::json!({"role_id":"rid","secret_id":"sid"}).to_string(),
            ))
            .respond_with(login_ok("tok-1", 300))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/kv/data/app/prod"))
            .and(header("X-Vault-Token", "tok-1"))
            .respond_with(kv_ok(
                serde_json::json!({"stripe":"sk-vault-1","other":"x"}),
            ))
            .mount(&server)
            .await;

        let p = provider(&server).await;
        let sref = SecretRef::parse("secret://vault/kv/app/prod#stripe").expect("ref");
        let v = p.get(&sref).await.expect("resolves");
        assert_eq!(v.expose(), "sk-vault-1");
    }

    #[tokio::test]
    async fn token_cached_across_reads() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/approle/login"))
            .respond_with(login_ok("tok-c", 300))
            .expect(1) // exactly one login for two reads
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/kv/data/a"))
            .respond_with(kv_ok(serde_json::json!({"v":"1"})))
            .expect(2)
            .mount(&server)
            .await;

        let p = provider(&server).await;
        let sref = SecretRef::parse("secret://vault/kv/a").expect("ref");
        p.get(&sref).await.expect("first");
        p.get(&sref).await.expect("second");
    }

    #[tokio::test]
    async fn forbidden_triggers_one_relogin_retry() {
        let server = MockServer::start().await;
        // First login returns a token the KV endpoint rejects; second works.
        Mock::given(method("POST"))
            .and(path("/v1/auth/approle/login"))
            .respond_with(login_ok("tok-x", 300))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/kv/data/b"))
            .respond_with(ResponseTemplate::new(403))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/kv/data/b"))
            .respond_with(kv_ok(serde_json::json!({"v":"after-retry"})))
            .mount(&server)
            .await;

        let p = provider(&server).await;
        let sref = SecretRef::parse("secret://vault/kv/b").expect("ref");
        let v = p.get(&sref).await.expect("resolves after retry");
        assert_eq!(v.expose(), "after-retry");
    }

    #[tokio::test]
    async fn not_found_and_multi_field_errors_name_no_values() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/approle/login"))
            .respond_with(login_ok("tok-n", 300))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/kv/data/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/kv/data/multi"))
            .respond_with(kv_ok(serde_json::json!({"a":"va","b":"vb"})))
            .mount(&server)
            .await;

        let p = provider(&server).await;

        let missing = SecretRef::parse("secret://vault/kv/missing").expect("ref");
        let err = p.get(&missing).await.expect_err("404");
        assert!(matches!(err, SecretError::NotFound { .. }));

        let multi = SecretRef::parse("secret://vault/kv/multi").expect("ref");
        let err = p.get(&multi).await.expect_err("ambiguous");
        let msg = err.to_string();
        assert!(msg.contains("#<key>"));
        assert!(
            !msg.contains("va") || msg.contains("\"a\""),
            "keys ok, values never: {msg}"
        );
        assert!(!msg.contains("vb"));
    }

    #[tokio::test]
    async fn failed_login_is_storage_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/approle/login"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;

        let p = provider(&server).await;
        let sref = SecretRef::parse("secret://vault/kv/x").expect("ref");
        let err = p.get(&sref).await.expect_err("login fails");
        assert!(err.to_string().contains("login"));
    }

    #[tokio::test]
    async fn missing_mount_segment_is_malformed() {
        let server = MockServer::start().await;
        let p = provider(&server).await;
        // `secret://vault/onlyone` parses (scheme+path) but lacks mount/path split.
        let sref = SecretRef::parse("secret://vault/onlyone").expect("ref");
        let err = p.get(&sref).await.expect_err("needs mount/path");
        assert!(matches!(err, SecretError::MalformedRef(_)));
    }
}
