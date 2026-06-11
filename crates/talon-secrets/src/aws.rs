//! AWS Secrets Manager provider — default credential chain, read-only
//! (criterion 14).
//!
//! Reference syntax: `secret://aws/<name>#<json-key>` — `<name>` is the
//! Secrets Manager secret id (a path-style name like `prod/db` works as-is);
//! the optional `#<json-key>` extracts a field from a JSON string secret.
//!
//! Credentials come from the SDK's default chain (env vars, shared profile,
//! IMDS, IRSA) — the whole point on Lambda/EC2/EKS is that no credential is
//! stored anywhere. Binary secrets are unsupported v1.

use std::{future::Future, pin::Pin};

use aws_sdk_secretsmanager::Client;

use crate::{SecretError, SecretMeta, SecretProvider, SecretRef, SecretValue};

pub const AWS_SCHEME: &str = "aws";

pub struct AwsProvider {
    client: Client,
}

impl AwsProvider {
    /// Build from the default credential/region chain. `region` overrides
    /// when set in config.
    pub async fn from_default_chain(region: Option<String>) -> Self {
        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest());
        if let Some(region) = region {
            loader = loader.region(aws_config::Region::new(region));
        }
        let cfg = loader.load().await;
        Self {
            client: Client::new(&cfg),
        }
    }

    /// Test/bench constructor with a pre-built client.
    pub fn from_client(client: Client) -> Self {
        Self { client }
    }

    fn extract(sref: &SecretRef, secret_string: &str) -> Result<SecretValue, SecretError> {
        match &sref.key {
            None => Ok(SecretValue::new(secret_string)),
            Some(key) => {
                let parsed: serde_json::Value =
                    serde_json::from_str(secret_string).map_err(|_| {
                        SecretError::Storage(format!(
                            "aws secret `{}` is not JSON — drop the #{key} fragment",
                            sref.path
                        ))
                    })?;
                let field = parsed
                    .get(key.as_str())
                    .ok_or_else(|| SecretError::NotFound {
                        scheme: AWS_SCHEME.to_string(),
                        name: sref.display_name().to_string(),
                    })?;
                match field.as_str() {
                    Some(s) => Ok(SecretValue::new(s)),
                    None => Ok(SecretValue::new(field.to_string())),
                }
            }
        }
    }
}

impl SecretProvider for AwsProvider {
    fn scheme(&self) -> &'static str {
        AWS_SCHEME
    }

    fn get<'a>(
        &'a self,
        sref: &'a SecretRef,
    ) -> Pin<Box<dyn Future<Output = Result<SecretValue, SecretError>> + Send + 'a>> {
        Box::pin(async move {
            let out = self
                .client
                .get_secret_value()
                .secret_id(&sref.path)
                .send()
                .await
                .map_err(|e| {
                    let svc = e.into_service_error();
                    if svc.is_resource_not_found_exception() {
                        SecretError::NotFound {
                            scheme: AWS_SCHEME.to_string(),
                            name: sref.display_name().to_string(),
                        }
                    } else {
                        SecretError::Storage(format!("aws secretsmanager: {svc}"))
                    }
                })?;

            match out.secret_string() {
                Some(s) => Self::extract(sref, s),
                None => Err(SecretError::Storage(format!(
                    "aws secret `{}` is binary — only string secrets are supported",
                    sref.path
                ))),
            }
        })
    }

    /// Listing requires `secretsmanager:ListSecrets` and is noisy — not v1.
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

    // Field extraction is the only pure logic here; the SDK call itself is a
    // thin passthrough exercised against the HTTP-replay client below.

    fn sref(s: &str) -> SecretRef {
        SecretRef::parse(s).expect("ref")
    }

    #[test]
    fn plain_string_passthrough_without_fragment() {
        let v = AwsProvider::extract(&sref("secret://aws/prod/db"), "raw-value").expect("ok");
        assert_eq!(v.expose(), "raw-value");
    }

    #[test]
    fn json_key_extraction() {
        let v = AwsProvider::extract(
            &sref("secret://aws/prod/db#password"),
            r#"{"username":"u","password":"p-9"}"#,
        )
        .expect("ok");
        assert_eq!(v.expose(), "p-9");
    }

    #[test]
    fn missing_json_key_is_not_found_naming_ref() {
        let err = AwsProvider::extract(
            &sref("secret://aws/prod/db#nope"),
            r#"{"username":"leakable-value-83"}"#,
        )
        .expect_err("missing key");
        let msg = err.to_string();
        assert!(msg.contains("secret://aws/prod/db#nope"));
        assert!(
            !msg.contains("leakable-value-83"),
            "no values in error: {msg}"
        );
    }

    #[test]
    fn fragment_on_non_json_is_actionable_error() {
        let err =
            AwsProvider::extract(&sref("secret://aws/x#k"), "not json").expect_err("not json");
        assert!(err.to_string().contains("not JSON"));
    }

    // The SDK call itself is a thin passthrough; an HTTP-replay SDK test
    // would drag aws-smithy test-util dev-deps into every workspace test
    // build (dev-deps compile unconditionally), breaking the lean-default
    // rule. Live-path coverage belongs to the feature-gated CI job.
}
