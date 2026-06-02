//! Static catalog of known LLM providers.
//!
//! This is pure data — the "long list" the onboarding wizard shows. Most
//! entries are OpenAI-compatible vendors that reuse [`crate::OpenAiCompatProvider`]
//! with a different `base_url`; only Anthropic and the key-less providers are
//! special-cased at construction time (see the binary's provider wiring).

use crate::ModelInfo;

/// A known provider the wizard can configure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderPreset {
    /// Stable lookup key persisted in config (e.g. `openrouter`).
    pub name: &'static str,
    /// Menu label.
    pub display_name: &'static str,
    /// API root without trailing slash. Empty for providers with bespoke wiring.
    pub base_url: &'static str,
    /// Env var holding the API key, or `None` for key-less (auth handled by the
    /// provider itself, e.g. `gh auth token`).
    pub key_env: Option<&'static str>,
    /// Whether [`crate::OpenAiCompatProvider`] can drive this endpoint directly.
    pub openai_compatible: bool,
    /// Whether models can be fetched live via a `/models` endpoint. When false,
    /// the wizard falls back to [`ProviderPreset::builtin_models`].
    pub lists_models: bool,
    /// Sensible default model id selected if the user doesn't pick one.
    pub default_model: &'static str,
    /// Built-in model list for providers without a live `/models` endpoint.
    pub builtin_models: &'static [&'static str],
}

impl ProviderPreset {
    /// `true` when the user must supply an API key (vs. key-less auth).
    pub fn needs_api_key(&self) -> bool {
        self.key_env.is_some()
    }

    /// Built-in models as [`ModelInfo`] — used when live listing is unavailable.
    pub fn builtin_model_infos(&self) -> Vec<ModelInfo> {
        self.builtin_models
            .iter()
            .map(|m| ModelInfo::new(*m))
            .collect()
    }
}

/// Full provider catalog.
pub fn presets() -> &'static [ProviderPreset] {
    PRESETS
}

/// Look up a preset by its stable `name`.
pub fn find(name: &str) -> Option<&'static ProviderPreset> {
    PRESETS.iter().find(|p| p.name == name)
}

const PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        name: "openai",
        display_name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        key_env: Some("OPENAI_API_KEY"),
        openai_compatible: true,
        lists_models: true,
        default_model: "gpt-4o-mini",
        builtin_models: &[],
    },
    ProviderPreset {
        name: "anthropic",
        display_name: "Anthropic",
        base_url: "https://api.anthropic.com/v1",
        key_env: Some("ANTHROPIC_API_KEY"),
        openai_compatible: false,
        lists_models: true,
        default_model: "claude-opus-4-7",
        builtin_models: &[],
    },
    ProviderPreset {
        name: "openrouter",
        display_name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        key_env: Some("OPENROUTER_API_KEY"),
        openai_compatible: true,
        lists_models: true,
        default_model: "openai/gpt-4o-mini",
        builtin_models: &[],
    },
    ProviderPreset {
        name: "nvidia",
        display_name: "NVIDIA NIM",
        base_url: "https://integrate.api.nvidia.com/v1",
        key_env: Some("NVIDIA_API_KEY"),
        openai_compatible: true,
        lists_models: true,
        default_model: "meta/llama-3.1-70b-instruct",
        builtin_models: &[],
    },
    ProviderPreset {
        name: "huggingface",
        display_name: "Hugging Face",
        base_url: "https://router.huggingface.co/v1",
        key_env: Some("HF_TOKEN"),
        openai_compatible: true,
        lists_models: true,
        default_model: "meta-llama/Llama-3.1-8B-Instruct",
        builtin_models: &[],
    },
    ProviderPreset {
        name: "together",
        display_name: "Together AI",
        base_url: "https://api.together.xyz/v1",
        key_env: Some("TOGETHER_API_KEY"),
        openai_compatible: true,
        lists_models: true,
        default_model: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        builtin_models: &[],
    },
    ProviderPreset {
        name: "groq",
        display_name: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        key_env: Some("GROQ_API_KEY"),
        openai_compatible: true,
        lists_models: true,
        default_model: "llama-3.3-70b-versatile",
        builtin_models: &[],
    },
    ProviderPreset {
        name: "deepseek",
        display_name: "DeepSeek",
        base_url: "https://api.deepseek.com/v1",
        key_env: Some("DEEPSEEK_API_KEY"),
        openai_compatible: true,
        lists_models: true,
        default_model: "deepseek-chat",
        builtin_models: &[],
    },
    // Key-less providers — auth handled by the provider, no live /models endpoint.
    ProviderPreset {
        name: "github-copilot",
        display_name: "GitHub Copilot",
        base_url: "https://api.githubcopilot.com",
        key_env: None,
        openai_compatible: false,
        lists_models: false,
        default_model: "claude-sonnet-4.6",
        builtin_models: &["claude-sonnet-4.6", "gpt-4o", "o1"],
    },
    ProviderPreset {
        name: "claude-code",
        display_name: "Claude Code",
        base_url: "",
        key_env: None,
        openai_compatible: false,
        lists_models: false,
        default_model: "claude-sonnet-4.6",
        builtin_models: &["claude-sonnet-4.6", "claude-opus-4-7"],
    },
    ProviderPreset {
        name: "codex",
        display_name: "OpenAI Codex",
        base_url: "",
        key_env: None,
        openai_compatible: false,
        lists_models: false,
        default_model: "gpt-5-codex",
        builtin_models: &["gpt-5-codex", "gpt-4o"],
    },
    ProviderPreset {
        name: "antigravity",
        display_name: "Antigravity",
        base_url: "",
        key_env: None,
        openai_compatible: false,
        lists_models: false,
        default_model: "gemini-2.0-flash",
        builtin_models: &["gemini-2.0-flash", "gemini-1.5-pro"],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_non_empty_and_unique() {
        assert!(presets().len() >= 10);
        let mut names: Vec<&str> = presets().iter().map(|p| p.name).collect();
        names.sort_unstable();
        let unique = names.len();
        names.dedup();
        assert_eq!(unique, names.len(), "provider names must be unique");
    }

    #[test]
    fn find_returns_known_provider() {
        assert_eq!(
            find("openrouter").map(|p| p.base_url),
            Some("https://openrouter.ai/api/v1")
        );
        assert!(find("does-not-exist").is_none());
    }

    #[test]
    fn core_providers_present() {
        for name in [
            "openai",
            "anthropic",
            "openrouter",
            "nvidia",
            "huggingface",
            "github-copilot",
        ] {
            assert!(find(name).is_some(), "missing preset: {name}");
        }
    }

    #[test]
    fn api_key_providers_declare_env_var() {
        let openai = find("openai").expect("openai");
        assert!(openai.needs_api_key());
        assert_eq!(openai.key_env, Some("OPENAI_API_KEY"));
    }

    #[test]
    fn keyless_providers_have_builtin_models() {
        for name in ["github-copilot", "claude-code", "codex", "antigravity"] {
            let p = find(name).expect("preset");
            assert!(!p.needs_api_key(), "{name} should be key-less");
            assert!(!p.lists_models, "{name} has no live /models");
            assert!(!p.builtin_models.is_empty(), "{name} needs a built-in list");
        }
    }

    #[test]
    fn builtin_model_infos_wraps_ids() {
        let infos = find("github-copilot")
            .expect("copilot")
            .builtin_model_infos();
        assert!(infos.iter().any(|m| m.id == "claude-sonnet-4.6"));
    }
}
