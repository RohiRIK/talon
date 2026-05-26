# Configuration Management

> **Status:** ✅ Complete
> **Category:** DevOps

---

## 1. Config File Location

```
~/.talon/profiles/<profile_name>/
├── config.toml         # Main configuration
└── .env                # Secrets (sourced on startup)
```

Multiple profiles allow running Talon with different personas,
models, or API keys:

```bash
talon --profile work     # → ~/.talon/profiles/work/config.toml
talon --profile personal # → ~/.talon/profiles/personal/config.toml
talon                    # → ~/.talon/profiles/default/config.toml
```

---

## 2. Full config.toml Reference

```toml
[agent]
system_prompt = "You are Talon, a direct and technically precise AI assistant."
max_iterations = 100
max_concurrent_sessions = 10

[llm]
default_provider = "anthropic"
default_model = "claude-sonnet-4-5"
temperature = 0.7

[llm.providers.anthropic]
type = "anthropic"
api_key = "${env:ANTHROPIC_API_KEY}"
model = "claude-sonnet-4-5"

[llm.providers.openai]
type = "openai_compat"
base_url = "https://api.openai.com/v1"
api_key = "${env:OPENAI_API_KEY}"
model = "gpt-4o"

[llm.providers.ollama]
type = "openai_compat"
base_url = "http://localhost:11434/v1"
api_key = "ollama"
model = "llama3.2"

[memory]
db_path = "~/.talon/profiles/default/db/talon.db"
fts_enabled = true
semantic_search = false
user_profile_path = "~/.talon/profiles/default/memories/USER.md"
agent_memory_path = "~/.talon/profiles/default/memories/MEMORY.md"
context_window_limit = 180000
history_messages = 50

[tools]
[tools.terminal]
enabled = true
timeout_secs = 180
max_output_bytes = 51200    # 50KB
allowed_commands = []       # empty = all allowed
blocked_commands = ["rm -rf /", "sudo rm"]

[tools.web_search]
enabled = true
provider = "brave"          # brave | serper | google | duckduckgo
api_key = "${env:BRAVE_API_KEY}"
results_per_query = 5

[tools.web_extract]
enabled = true
timeout_secs = 30
max_urls_per_call = 5

[tools.code_exec]
enabled = false             # Disabled by default (sandboxed execution)
sandbox = "docker"

[tools.delegate_task]
enabled = true
max_spawn_depth = 1
max_concurrent = 3

[gateway.telegram]
enabled = true
bot_token = "${env:TELEGRAM_BOT_TOKEN}"
home_chat_id = "${env:TELEGRAM_HOME_CHAT_ID}"
allowed_user_ids = []       # empty = all users
mode = "polling"            # polling | webhook
polling_timeout = 30

[gateway.http]
enabled = false
bind = "0.0.0.0:8080"
auth_token = "${env:HTTP_AUTH_TOKEN}"

[log]
level = "info"              # trace | debug | info | warn | error
format = "pretty"           # pretty | json
file = "~/.talon/logs/talon.log"
max_size_mb = 100
keep_files = 7

[tts]
enabled = false
provider = "edge"           # edge | openai
voice = "en-US-AriaNeural"
```

---

## 3. Environment Variable Interpolation

Values of the form `${env:VAR_NAME}` are resolved at startup:

```rust
pub fn resolve_env_vars(value: &str) -> Result<String, ConfigError> {
    let re = Regex::new(r"\$\{env:([A-Z_][A-Z0-9_]*)\}").unwrap();
    let mut result = value.to_string();

    for cap in re.captures_iter(value) {
        let var_name = &cap[1];
        let var_value = std::env::var(var_name)
            .map_err(|_| ConfigError::MissingEnvVar(var_name.to_string()))?;
        result = result.replace(&cap[0], &var_value);
    }

    Ok(result)
}
```

---

## 4. .env File

```bash
# ~/.talon/profiles/default/.env
ANTHROPIC_API_KEY=sk-ant-...
TELEGRAM_BOT_TOKEN=123456:...
TELEGRAM_HOME_CHAT_ID=792555016
BRAVE_API_KEY=...
OPENAI_API_KEY=sk-...
```

Talon loads `.env` via `dotenvy` at startup:
```rust
dotenvy::from_path(profile_dir.join(".env")).ok();  // Ignore if missing
```

---

## 5. Config Validation

At startup, Talon validates the config and reports helpful errors:

```rust
pub fn validate(config: &Config) -> Result<(), Vec<ConfigError>> {
    let mut errors = vec![];

    // Must have at least one LLM provider
    if config.llm.providers.is_empty() {
        errors.push(ConfigError::NoLlmProvider);
    }

    // Must have at least one gateway enabled
    let gateways_enabled = config.gateway.telegram.enabled
        || config.gateway.http.enabled
        || config.gateway.discord.enabled;

    if !gateways_enabled {
        errors.push(ConfigError::NoGatewayEnabled);
    }

    // Validate API keys
    if config.gateway.telegram.enabled && config.gateway.telegram.bot_token.is_empty() {
        errors.push(ConfigError::MissingApiKey("TELEGRAM_BOT_TOKEN".into()));
    }

    if errors.is_empty() { Ok(()) } else { Err(errors) }
}
```
---

## Related Documents

### Depends On
- [Config System](../02_Architecture/18a_Config_System.md)

### See Also
- [Profile Isolation](../04_Core_Features/40_Profile_Isolation.md)
- [Security Model](../02_Architecture/20_Security_Model.md)

