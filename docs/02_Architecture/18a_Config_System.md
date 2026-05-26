# Configuration System

> **Last corrected:** dogfood pass 4

> **Status:** ✅ Complete
> **Category:** Architecture

---

## 1. Design Goals

1. **Type-safe at compile time** — config structs derived via `serde`
2. **Multiple sources, clear precedence** — file → env vars → CLI flags
3. **Sensible defaults** — an empty config file is valid; nothing is mandatory except at least one LLM provider
4. **Secret isolation** — API keys never written to disk by Talon; read from env or secret store
5. **Hot-reload friendly** — tools and skills reload without restart; core config requires restart

---

## 2. Config File Format

Talon uses TOML. Default location: `~/.talon/config.toml` (Linux/macOS)
or `%APPDATA%\talon\config.toml` (Windows).

```toml
# config.toml — full annotated example

[agent]
name = "Talon"
default_provider = "anthropic"
default_model = "claude-opus-4"
max_iterations = 50         # safety limit per agent run
approval_mode = "ask_for_dangerous"  # always_ask | ask_for_dangerous | always_approve

[agent.system_prompt]
# Inline or path to file
text = "You are Talon, a capable AI agent..."
# path = "~/.talon/system_prompt.md"   # alternative

[providers.anthropic]
api_key_env = "ANTHROPIC_API_KEY"       # env var name, NOT the key itself
base_url = "https://api.anthropic.com"  # optional override
default_model = "claude-opus-4"
max_tokens = 8192
timeout_secs = 120

[providers.openai]
api_key_env = "OPENAI_API_KEY"
default_model = "gpt-4o"
max_tokens = 4096

[providers.openrouter]
api_key_env = "OPENROUTER_API_KEY"
base_url = "https://openrouter.ai/api/v1"
default_model = "meta-llama/llama-3.1-405b-instruct"

[providers.ollama]
base_url = "http://localhost:11434"
default_model = "llama3.2"
# No API key needed

[memory]
data_dir = "~/.talon/data"         # SQLite + skills + logs live here
db_filename = "talon.db"

[memory.embeddings]
enabled = false
model = "nomic-embed-text"
dim = 768
backend = "sqlite_vec"              # sqlite_vec | qdrant

[tools]
# Per-tool approval overrides (default from agent.approval_mode)
[tools.overrides]
terminal_execute = "ask_for_dangerous"
file_write = "ask_for_dangerous"
web_search = "always_approve"
send_message = "always_approve"

[tools.terminal]
timeout_secs = 30
sandbox = true                  # wrap in Docker sandbox if available

[tools.browser]
headless = true
chromium_path = ""              # auto-detect if empty

[gateway]
# Which interfaces to enable
[gateway.telegram]
enabled = false
bot_token_env = "TELEGRAM_BOT_TOKEN"
home_chat_id_env = "TELEGRAM_HOME_CHAT_ID"

[gateway.http]
enabled = true
bind = "127.0.0.1:8080"
auth_token_env = "TALON_HTTP_TOKEN"  # bearer token, empty = no auth

[gateway.cli]
enabled = true

[logging]
level = "info"                  # trace|debug|info|warn|error
format = "pretty"               # pretty | json | compact
file = ""                       # write logs to file (empty = stderr only)
```

---

## 3. Config Struct (Rust)

```rust
// talon-core/src/config.rs

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TalonConfig {
    pub agent: AgentConfig,
    pub providers: HashMap<String, ProviderConfig>,
    pub memory: MemoryConfig,
    pub tools: ToolsConfig,
    pub gateway: GatewayConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentConfig {
    pub name: String,
    pub default_provider: String,
    pub default_model: String,
    pub max_iterations: u32,
    pub approval_mode: ApprovalLevel,
    pub system_prompt: SystemPromptConfig,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: "Talon".into(),
            default_provider: "anthropic".into(),
            default_model: "claude-opus-4".into(),
            max_iterations: 50,
            approval_mode: ApprovalLevel::Confirmation,
            system_prompt: SystemPromptConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SystemPromptConfig {
    Inline { text: String },
    FromFile { path: String },
}

impl Default for SystemPromptConfig {
    fn default() -> Self {
        Self::Inline { text: String::new() }
    }
}

impl SystemPromptConfig {
    pub fn resolve(&self) -> anyhow::Result<String> {
        match self {
            Self::Inline { text } => Ok(text.clone()),
            Self::FromFile { path } => {
                let p = shellexpand::tilde(path);
                Ok(std::fs::read_to_string(p.as_ref())?)
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
    pub max_tokens: Option<u32>,
    pub timeout_secs: Option<u64>,
}

impl ProviderConfig {
    /// Resolve the API key from env at runtime
    pub fn resolve_api_key(&self) -> anyhow::Result<Option<String>> {
        match &self.api_key_env {
            Some(env_name) => {
                std::env::var(env_name)
                    .map(Some)
                    .map_err(|_| anyhow::anyhow!(
                        "API key env var '{}' not set", env_name
                    ))
            }
            None => Ok(None),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct MemoryConfig {
    pub data_dir: Option<String>,
    pub db_filename: String,
    pub embeddings: EmbeddingsConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct EmbeddingsConfig {
    pub enabled: bool,
    pub model: Option<String>,
    pub dim: Option<usize>,
    pub backend: EmbeddingBackend,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingBackend {
    #[default]
    SqliteVec,
    Qdrant,
}
```

---

## 4. Loading with Precedence

```rust
pub fn load_config(cli_config_path: Option<&str>) -> anyhow::Result<TalonConfig> {
    use config::{Config, Environment, File};

    let config_path = cli_config_path
        .map(String::from)
        .or_else(|| std::env::var("TALON_CONFIG").ok())
        .unwrap_or_else(default_config_path);

    let config = Config::builder()
        // 1. Defaults (via serde default impls)
        .add_source(File::with_name(&config_path).required(false))
        // 2. Environment variables: TALON_AGENT__NAME → agent.name
        .add_source(
            Environment::with_prefix("TALON")
                .separator("__")
                .try_parsing(true)
        )
        .build()?;

    let cfg: TalonConfig = config.try_deserialize()?;

    validate_config(&cfg)?;

    Ok(cfg)
}

fn validate_config(cfg: &TalonConfig) -> anyhow::Result<()> {
    // At least one provider must exist
    if cfg.providers.is_empty() {
        anyhow::bail!(
            "No LLM providers configured. Add at least one [providers.*] section."
        );
    }

    // Default provider must exist in providers map
    if !cfg.providers.contains_key(&cfg.agent.default_provider) {
        anyhow::bail!(
            "default_provider '{}' not found in [providers]",
            cfg.agent.default_provider
        );
    }

    Ok(())
}

fn default_config_path() -> String {
    let home = dirs::home_dir().unwrap_or_default();
    home.join(".talon").join("config.toml")
        .to_string_lossy()
        .into_owned()
}
```

---

## 5. CLI Overrides

CLI flags take highest precedence:

```rust
#[derive(Parser)]
pub struct Cli {
    /// Path to config file
    #[arg(long, short, env = "TALON_CONFIG")]
    pub config: Option<String>,

    /// Override LLM provider (e.g., anthropic, openai, ollama)
    #[arg(long)]
    pub provider: Option<String>,

    /// Override model
    #[arg(long)]
    pub model: Option<String>,

    /// Override approval mode
    #[arg(long, value_enum)]
    pub approval_mode: Option<ApprovalMode>,

    #[command(subcommand)]
    pub command: Command,
}

pub fn apply_cli_overrides(cfg: &mut TalonConfig, cli: &Cli) {
    if let Some(p) = &cli.provider {
        cfg.agent.default_provider = p.clone();
    }
    if let Some(m) = &cli.model {
        cfg.agent.default_model = m.clone();
    }
    if let Some(a) = &cli.approval_mode {
        cfg.agent.approval_mode = *a;
    }
}
```

---

## 6. Precedence Summary

```
(lowest)  serde #[serde(default)] embedded in structs
            ↓
          config.toml file
            ↓
          TALON__* environment variables
            ↓
(highest) CLI --flags
```

API keys are **never** in config.toml — always read from env at runtime.
This ensures secrets don't end up in version control.
---

## Related Documents

### Depends On
- [Cargo Workspace Design](12_Workspace_And_Crate_Structure.md)

### Used By
- [Profile Isolation](../04_Core_Features/40_Profile_Isolation.md)
- [Configuration Management](../08_DevOps/63_Configuration_Management.md)

### See Also
- [Gateway Architecture](18_Gateway_MultiChannel_Architecture.md)

