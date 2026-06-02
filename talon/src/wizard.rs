//! Interactive provider/model onboarding wizard (W6).
//!
//! Driven by `inquire` — an interactive terminal UI (arrow-key menus,
//! multi-select, masked key entry). Flow: pick providers from the preset
//! catalog → enter keys (stored in the OS keychain, never the config) →
//! live-fetch models where the provider supports it → choose a default model
//! per provider → order the fallback chain. The result is an [`LlmConfig`]
//! whose head is the primary provider and whose tail is the fallback order.

use anyhow::{Context, Result};
use inquire::{MultiSelect, Password, Select};
use talon_llm::{
    LlmConfig, ModelInfo, ModelLister, OpenAiCompatProvider, ProviderChoice, presets,
};

/// Build an [`LlmConfig`] from ordered `(provider, model)` picks. The first
/// entry is the primary provider; the rest are the fallback order.
pub fn build_chain(selections: &[(String, String)]) -> LlmConfig {
    let chain = selections
        .iter()
        .map(|(provider, model)| ProviderChoice {
            provider: provider.clone(),
            model: Some(model.clone()),
            base_url: None,
        })
        .collect();
    LlmConfig { chain }
}

/// Merge an `[llm]` chain into an existing config.toml, preserving every other
/// section. Any prior `[llm]` table is replaced.
pub fn merge_llm_into_config(existing: &str, cfg: &LlmConfig) -> Result<String> {
    let mut doc: toml::Table = toml::from_str(existing).unwrap_or_default();
    let llm_val = toml::Value::try_from(cfg).context("serialize [llm] config")?;
    doc.insert("llm".to_string(), llm_val);
    toml::to_string_pretty(&doc).context("serialize merged config")
}

/// Live-fetch the model list for a provider, degrading gracefully: live
/// `/models` for OpenAI-compatible providers, else the preset's built-in list,
/// else the single default model.
async fn fetch_models(preset: &presets::ProviderPreset, key: &str) -> Vec<ModelInfo> {
    if preset.openai_compatible && preset.lists_models && !key.is_empty() {
        let provider = OpenAiCompatProvider::new(
            preset.base_url.to_string(),
            key.to_string(),
            preset.default_model.to_string(),
        );
        match provider.list_models().await {
            Ok(models) if !models.is_empty() => return models,
            Ok(_) => {}
            Err(e) => tracing::warn!("model fetch for {} failed: {e}", preset.name),
        }
    }
    let builtin = preset.builtin_model_infos();
    if builtin.is_empty() {
        vec![ModelInfo::new(preset.default_model)]
    } else {
        builtin
    }
}

/// Run the interactive wizard. Returns `None` when the user selects no
/// providers. Stores entered API keys in the OS keychain as a side effect.
pub async fn run_provider_wizard() -> Result<Option<LlmConfig>> {
    let all = presets::presets();
    let labels: Vec<&str> = all.iter().map(|p| p.display_name).collect();

    let chosen = MultiSelect::new("Select providers to configure:", labels)
        .prompt()
        .context("provider selection cancelled")?;
    if chosen.is_empty() {
        return Ok(None);
    }

    let chosen_presets: Vec<&presets::ProviderPreset> = chosen
        .iter()
        .filter_map(|label| all.iter().find(|p| p.display_name == *label))
        .collect();

    let mut selections: Vec<(String, String)> = Vec::new();
    for preset in &chosen_presets {
        let key = if preset.needs_api_key() {
            let entered = Password::new(&format!("API key for {}:", preset.display_name))
                .without_confirmation()
                .prompt()
                .context("key entry cancelled")?;
            if !entered.is_empty() {
                crate::store_provider_key(preset.name, &entered)
                    .with_context(|| format!("failed to store {} key", preset.name))?;
            }
            entered
        } else {
            String::new()
        };

        let models = fetch_models(preset, &key).await;
        let model_ids: Vec<String> = models.into_iter().map(|m| m.id).collect();
        let cursor = model_ids
            .iter()
            .position(|id| id == preset.default_model)
            .unwrap_or(0);
        let model = Select::new(
            &format!("Default model for {}:", preset.display_name),
            model_ids,
        )
        .with_starting_cursor(cursor)
        .prompt()
        .context("model selection cancelled")?;

        selections.push((preset.name.to_string(), model));
    }

    if selections.len() > 1 {
        let names: Vec<String> = selections.iter().map(|(n, _)| n.clone()).collect();
        let primary = Select::new("Primary provider (the rest are fallbacks, in order):", names)
            .prompt()
            .context("primary selection cancelled")?;
        if let Some(pos) = selections.iter().position(|(n, _)| n == &primary) {
            let head = selections.remove(pos);
            selections.insert(0, head);
        }
    }

    Ok(Some(build_chain(&selections)))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn build_chain_makes_head_primary() {
        let cfg = build_chain(&[
            ("openrouter".into(), "openai/gpt-4o".into()),
            ("groq".into(), "llama-3.3-70b-versatile".into()),
        ]);
        assert_eq!(cfg.chain.len(), 2);
        assert_eq!(cfg.primary().map(|c| c.provider.as_str()), Some("openrouter"));
        assert_eq!(cfg.chain[0].model.as_deref(), Some("openai/gpt-4o"));
        assert_eq!(cfg.chain[1].provider, "groq");
    }

    #[test]
    fn merge_preserves_other_sections_and_replaces_llm() {
        let existing = r#"
[llm]
provider = "anthropic"
model = ""

[memory]
db_path = "talon.db"
"#;
        let cfg = build_chain(&[("github-copilot".into(), "claude-sonnet-4.6".into())]);
        let merged = merge_llm_into_config(existing, &cfg).expect("merge");
        // Old flat [llm] keys gone, chain present, [memory] untouched.
        assert!(merged.contains("[[llm.chain]]"));
        assert!(merged.contains("github-copilot"));
        assert!(merged.contains("db_path"));
        assert!(!merged.contains("provider = \"anthropic\""));
        // Re-parse round-trips to the same chain.
        assert_eq!(LlmConfig::parse(&merged), cfg);
    }

    #[test]
    fn merge_into_empty_config_yields_only_llm() {
        let cfg = build_chain(&[("openai".into(), "gpt-4o-mini".into())]);
        let merged = merge_llm_into_config("", &cfg).expect("merge");
        assert_eq!(LlmConfig::parse(&merged), cfg);
    }
}
