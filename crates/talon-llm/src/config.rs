//! `[llm]` config — the ordered provider chain the wizard writes.
//!
//! The head of `chain` is the primary provider; the tail is the fallback
//! order tried in turn (see [`crate::FallbackProvider`]). Each entry names a
//! preset (`provider`) with optional `model` and `base_url` overrides. API
//! keys never live here — they are resolved from the OS keychain / env.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// One link in the provider chain: a preset name plus optional overrides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderChoice {
    /// Preset key (e.g. `openrouter`); see [`crate::presets`].
    pub provider: String,
    /// Model id override; falls back to the preset's `default_model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// API root override; falls back to the preset's `base_url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

impl ProviderChoice {
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: None,
            base_url: None,
        }
    }
}

/// The `[llm]` table — an ordered chain of providers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Head = primary, tail = fallback order.
    #[serde(default)]
    pub chain: Vec<ProviderChoice>,
}

#[derive(Debug, Default, Deserialize)]
struct RootConfig {
    #[serde(default)]
    llm: LlmConfig,
}

#[derive(Debug, Serialize)]
struct RootConfigOut<'a> {
    llm: &'a LlmConfig,
}

impl LlmConfig {
    /// Parse `[llm]` out of a full config.toml string. Unknown sections are
    /// ignored; an absent or invalid section yields an empty chain.
    pub fn parse(toml_str: &str) -> Self {
        match toml::from_str::<RootConfig>(toml_str) {
            Ok(root) => root.llm,
            Err(e) => {
                tracing::warn!("invalid [llm] config, using defaults: {e}");
                Self::default()
            }
        }
    }

    /// Load from a config file; a missing file yields an empty chain.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::parse(&s),
            Err(_) => Self::default(),
        }
    }

    /// Serialize just the `[llm]` section for the wizard to write.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(&RootConfigOut { llm: self })
    }

    pub fn is_empty(&self) -> bool {
        self.chain.is_empty()
    }

    /// The primary provider — head of the chain.
    pub fn primary(&self) -> Option<&ProviderChoice> {
        self.chain.first()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_is_empty() {
        let cfg = LlmConfig::load(Path::new("/no/such/config.toml"));
        assert!(cfg.is_empty());
        assert!(cfg.primary().is_none());
    }

    #[test]
    fn parses_llm_chain_ignoring_other_sections() {
        let toml = r#"
            [tools.web]
            search_backends = ["brave"]

            [[llm.chain]]
            provider = "github-copilot"

            [[llm.chain]]
            provider = "openrouter"
            model = "openai/gpt-4o"
        "#;
        let cfg = LlmConfig::parse(toml);
        assert_eq!(cfg.chain.len(), 2);
        assert_eq!(
            cfg.primary().map(|c| c.provider.as_str()),
            Some("github-copilot")
        );
        assert_eq!(cfg.chain[1].model.as_deref(), Some("openai/gpt-4o"));
    }

    #[test]
    fn round_trips_through_toml() {
        let cfg = LlmConfig {
            chain: vec![
                ProviderChoice::new("anthropic"),
                ProviderChoice {
                    provider: "openrouter".to_string(),
                    model: Some("openai/gpt-4o".to_string()),
                    base_url: Some("https://example/v1".to_string()),
                },
            ],
        };
        let serialized = cfg.to_toml().expect("serialize");
        let parsed = LlmConfig::parse(&serialized);
        assert_eq!(cfg, parsed);
    }

    #[test]
    fn omits_none_overrides_in_output() {
        let cfg = LlmConfig {
            chain: vec![ProviderChoice::new("anthropic")],
        };
        let serialized = cfg.to_toml().expect("serialize");
        assert!(!serialized.contains("model"));
        assert!(!serialized.contains("base_url"));
    }

    #[test]
    fn invalid_toml_yields_default() {
        let cfg = LlmConfig::parse("this is not = valid = toml = at all");
        assert!(cfg.is_empty());
    }
}
