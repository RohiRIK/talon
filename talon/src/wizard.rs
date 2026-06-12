//! Interactive provider/model onboarding wizard (W6).
//!
//! Driven by `inquire` — an interactive terminal UI (arrow-key menus, masked
//! key entry). Flow: pick the primary provider → enter its key (stored in the
//! OS keychain, never the config) → live-fetch models where the provider
//! supports it → choose a default model → then repeatedly offer to add a
//! fallback provider, configured the same way. The result is an [`LlmConfig`]
//! whose head is the primary provider and whose tail is the fallback order.

use anyhow::{Context, Result};
use inquire::{Confirm, Password, Select};
use talon_llm::{LlmConfig, ModelInfo, ModelLister, OpenAiCompatProvider, ProviderChoice, presets};

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

/// Run the interactive wizard. Returns `None` when the user selects no primary
/// provider. Stores entered API keys in the OS keychain as a side effect.
pub async fn run_provider_wizard() -> Result<Option<LlmConfig>> {
    let all = presets::presets();

    let primary = match select_provider(all, "Select your primary provider:", &[])? {
        Some(p) => p,
        None => return Ok(None),
    };
    let mut selections: Vec<(String, String)> = vec![configure_provider(primary).await?];

    // Offer fallbacks one at a time, appended in the order added. Each is tried
    // after the ones before it if they fail (see `FallbackProvider`).
    loop {
        let add = Confirm::new("Add a fallback provider?")
            .with_default(false)
            .prompt()
            .context("fallback prompt cancelled")?;
        if !add {
            break;
        }
        let chosen: Vec<&str> = selections.iter().map(|(n, _)| n.as_str()).collect();
        match select_provider(all, "Select a fallback provider:", &chosen)? {
            Some(preset) => selections.push(configure_provider(preset).await?),
            None => break,
        }
    }

    Ok(Some(build_chain(&selections)))
}

/// Prompt the user to pick one provider by display name, excluding any whose
/// `name` is in `exclude`. Returns `None` when nothing is left to choose.
fn select_provider<'a>(
    all: &'a [presets::ProviderPreset],
    prompt: &str,
    exclude: &[&str],
) -> Result<Option<&'a presets::ProviderPreset>> {
    let options: Vec<&str> = all
        .iter()
        .filter(|p| !exclude.contains(&p.name))
        .map(|p| p.display_name)
        .collect();
    if options.is_empty() {
        return Ok(None);
    }
    let label = Select::new(prompt, options)
        .prompt()
        .context("provider selection cancelled")?;
    Ok(all.iter().find(|p| p.display_name == label))
}

/// Configure one provider: enter its key (→ keychain), pick a default model.
/// Returns `(provider name, model id)`.
async fn configure_provider(preset: &presets::ProviderPreset) -> Result<(String, String)> {
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

    Ok((preset.name.to_string(), model))
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
        assert_eq!(
            cfg.primary().map(|c| c.provider.as_str()),
            Some("openrouter")
        );
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

    // ── fetch_models degradation (no network) ──────────────────────────────

    #[tokio::test]
    async fn fetch_models_uses_builtin_for_keyless_provider() {
        // github-copilot is not openai_compatible and ships a built-in list —
        // no network call regardless of key.
        let preset = presets::find("github-copilot").expect("preset");
        let ids: Vec<String> = fetch_models(preset, "")
            .await
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert!(ids.contains(&"claude-sonnet-4.6".to_string()));
    }

    #[tokio::test]
    async fn fetch_models_falls_back_to_default_when_no_builtin() {
        // anthropic isn't openai_compatible and has no built-in list → the
        // single default model, no network.
        let preset = presets::find("anthropic").expect("preset");
        let ids: Vec<String> = fetch_models(preset, "")
            .await
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, vec![preset.default_model.to_string()]);
    }

    #[tokio::test]
    async fn fetch_models_skips_live_fetch_when_key_empty() {
        // openrouter is openai_compatible but with an empty key the live fetch
        // is skipped (guarded), so it degrades to the default model offline.
        let preset = presets::find("openrouter").expect("preset");
        let ids: Vec<String> = fetch_models(preset, "")
            .await
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, vec![preset.default_model.to_string()]);
    }
}
