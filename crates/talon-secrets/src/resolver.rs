//! `SecretResolver` — just-in-time resolution of secret references in text.
//!
//! Called at job dispatch (criteria 10–11): the resolved text goes to the
//! LLM/tool context only; callers must register every resolved value with the
//! redaction registry so no sink ever sees plaintext. Errors carry the
//! reference name, never a value.

use std::collections::HashMap;
use std::sync::Arc;

use crate::{SecretError, SecretProvider, SecretRef, SecretValue};

/// Result of resolving a text: substituted text + every (name, value) pair,
/// for redaction registration.
pub struct Resolved {
    pub text: String,
    /// `(display_name, value)` for each distinct reference that resolved.
    pub values: Vec<(String, SecretValue)>,
}

/// `text` holds resolved plaintext — `Debug` elides it.
impl std::fmt::Debug for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolved")
            .field("text", &"[REDACTED]")
            .field("values", &self.values.len())
            .finish()
    }
}

#[derive(Default)]
pub struct SecretResolver {
    providers: HashMap<&'static str, Arc<dyn SecretProvider>>,
}

impl SecretResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a provider under its scheme; later registrations win.
    pub fn register(&mut self, provider: Arc<dyn SecretProvider>) {
        self.providers.insert(provider.scheme(), provider);
    }

    pub fn has_scheme(&self, scheme: &str) -> bool {
        self.providers.contains_key(scheme)
    }

    /// Resolve every reference in `text`. Fail-closed: the first unresolvable
    /// reference aborts the whole resolution (criterion 11 — no partial
    /// dispatch).
    pub async fn resolve_all(&self, text: &str) -> Result<Resolved, SecretError> {
        let refs = SecretRef::find_all(text);
        if refs.is_empty() {
            return Ok(Resolved {
                text: text.to_string(),
                values: vec![],
            });
        }

        let mut out = text.to_string();
        let mut values: Vec<(String, SecretValue)> = Vec::new();
        let mut seen: HashMap<String, SecretValue> = HashMap::new();

        for sref in refs {
            let value = match seen.get(&sref.raw) {
                Some(v) => v.clone(),
                None => {
                    let provider = self.providers.get(sref.scheme.as_str()).ok_or_else(|| {
                        SecretError::UnknownScheme(sref.scheme.clone())
                    })?;
                    let v = provider.get(&sref).await?;
                    seen.insert(sref.raw.clone(), v.clone());
                    values.push((sref.display_name().to_string(), v.clone()));
                    v
                }
            };
            out = out.replace(&sref.raw, value.expose());
        }

        Ok(Resolved { text: out, values })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::{future::Future, pin::Pin};

    struct MapProvider {
        scheme: &'static str,
        entries: Vec<(&'static str, &'static str)>,
    }

    impl SecretProvider for MapProvider {
        fn scheme(&self) -> &'static str {
            self.scheme
        }
        fn get<'a>(
            &'a self,
            sref: &'a SecretRef,
        ) -> Pin<Box<dyn Future<Output = Result<SecretValue, SecretError>> + Send + 'a>> {
            Box::pin(async move {
                self.entries
                    .iter()
                    .find(|(k, _)| *k == sref.path)
                    .map(|(_, v)| SecretValue::new(*v))
                    .ok_or_else(|| SecretError::NotFound {
                        scheme: self.scheme.to_string(),
                        name: sref.display_name().to_string(),
                    })
            })
        }
        fn list(
            &self,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<crate::SecretMeta>, SecretError>> + Send + '_>>
        {
            Box::pin(async { Ok(vec![]) })
        }
    }

    fn resolver() -> SecretResolver {
        let mut r = SecretResolver::new();
        r.register(Arc::new(MapProvider {
            scheme: "builtin",
            entries: vec![("API_KEY", "k-123"), ("DB_PASS", "p-456")],
        }));
        r.register(Arc::new(MapProvider {
            scheme: "vaultx",
            entries: vec![("kv/app", "v-789")],
        }));
        r
    }

    #[tokio::test]
    async fn resolves_multiple_refs_and_collects_values() {
        let r = resolver();
        let resolved = r
            .resolve_all("use {{secret:API_KEY}} and secret://vaultx/kv/app done")
            .await
            .expect("resolves");
        assert_eq!(resolved.text, "use k-123 and v-789 done");
        assert_eq!(resolved.values.len(), 2);
    }

    #[tokio::test]
    async fn repeated_ref_resolved_once_substituted_everywhere() {
        let r = resolver();
        let resolved = r
            .resolve_all("{{secret:API_KEY}} then {{secret:API_KEY}}")
            .await
            .expect("resolves");
        assert_eq!(resolved.text, "k-123 then k-123");
        assert_eq!(resolved.values.len(), 1, "deduped");
    }

    #[tokio::test]
    async fn unknown_name_fails_closed_naming_ref_only() {
        let r = resolver();
        let err = r
            .resolve_all("{{secret:API_KEY}} and {{secret:MISSING}}")
            .await
            .expect_err("must fail closed");
        let msg = err.to_string();
        assert!(msg.contains("MISSING"));
        assert!(!msg.contains("k-123"), "no resolved value in error");
    }

    #[tokio::test]
    async fn unknown_scheme_is_typed_error() {
        let r = resolver();
        let err = r
            .resolve_all("secret://nope/path")
            .await
            .expect_err("unknown scheme");
        assert!(matches!(err, SecretError::UnknownScheme(s) if s == "nope"));
    }

    #[tokio::test]
    async fn text_without_refs_passes_through() {
        let r = resolver();
        let resolved = r.resolve_all("plain text").await.expect("ok");
        assert_eq!(resolved.text, "plain text");
        assert!(resolved.values.is_empty());
    }
}
