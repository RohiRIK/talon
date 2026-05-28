# Claude Code Provider

> **Status:** 🚧 Planned — Phase 1.5 task 1.5.2
> **Category:** API Bindings

---

## 1. Scope

Claude Code (Anthropic's CLI) supports multiple auth paths. `ClaudeCodeProvider` resolves
credentials through the full Claude Code auth chain, preferring the OAuth token path. This
differs from the plain `AnthropicProvider` (which only accepts `ANTHROPIC_API_KEY`).

Auth chain (resolved once at construction):
1. `CLAUDE_CODE_OAUTH_TOKEN` env var → sent as `Authorization: Bearer`
2. `ANTHROPIC_AUTH_TOKEN` env var → sent as `Authorization: Bearer`
3. `ANTHROPIC_API_KEY` env var → sent as `x-api-key`
4. `claude setup-token` CLI subprocess → output used as Bearer token
5. `LlmError::AuthFailed` if all fail

---

## 2. Struct & Auth

```rust
// crates/talon-llm/src/claude_code.rs
// Feature flag: claude-code-provider

enum AnthropicAuth {
    Bearer(String),   // OAUTH_TOKEN, AUTH_TOKEN, or CLI-generated token
    ApiKey(String),   // ANTHROPIC_API_KEY — sent as x-api-key header
}

pub struct ClaudeCodeProvider {
    client: reqwest::Client,
    auth: AnthropicAuth,
    model: String,
}

impl ClaudeCodeProvider {
    const ENDPOINT: &'static str = "https://api.anthropic.com/v1/messages";
    const DEFAULT_MODEL: &'static str = "claude-opus-4-7";

    pub fn new() -> Result<Self, LlmError>;

    fn resolve_auth() -> Result<AnthropicAuth, LlmError> {
        // 1. CLAUDE_CODE_OAUTH_TOKEN → Bearer
        // 2. ANTHROPIC_AUTH_TOKEN → Bearer
        // 3. ANTHROPIC_API_KEY → ApiKey
        // 4. claude setup-token subprocess → Bearer
        // 5. AuthFailed
    }
}
```

The `AnthropicAuth` enum drives which header is sent per request:
- `Bearer(tok)` → `Authorization: Bearer {tok}`
- `ApiKey(key)` → `x-api-key: {key}`

---

## 3. Request Format

Anthropic Messages API format — identical to `AnthropicProvider`:

```json
{
  "model": "claude-opus-4-7",
  "max_tokens": 4096,
  "messages": [...],
  "tools": [...]
}
```

Additional required header: `anthropic-version: 2023-06-01`.

Response shape: `{ content: [ContentBlock], stop_reason: String }` — shares `ContentBlock`
from `talon-llm/src/lib.rs`.

---

## 4. Config

| Setting | Source | Default |
|---------|--------|---------|
| OAuth token | `CLAUDE_CODE_OAUTH_TOKEN` | — |
| Auth token | `ANTHROPIC_AUTH_TOKEN` | — |
| API key | `ANTHROPIC_API_KEY` | — |
| CLI fallback | `claude setup-token` stdout | — |
| Model | `TALON_LLM_MODEL` env var | `claude-opus-4-7` |
| Endpoint | `ANTHROPIC_BASE_URL` + `/v1/messages` | `https://api.anthropic.com/v1/messages` |

Feature flag: `features = ["claude-code-provider"]` in `talon-llm/Cargo.toml`.

---

## 5. Acceptance Criteria

- `CLAUDE_CODE_OAUTH_TOKEN` takes priority as Bearer token
- `ANTHROPIC_AUTH_TOKEN` used when OAUTH_TOKEN absent
- `ANTHROPIC_API_KEY` used (as `x-api-key`) when both Bearer sources absent
- `claude setup-token` CLI shelled out as last resort; its stdout used as Bearer token
- `LlmError::AuthFailed` if all four sources fail
- `Arc<dyn LlmProvider>` constructible
- No network calls at construction

---

## Related Documents

### Depends On
- [LLM Provider Abstraction](41_LLM_Provider_Abstraction.md)
- [Anthropic Provider](43_Anthropic_Provider.md)
- [Anthropic API Integration](43a_Anthropic_API_Integration.md)

### See Also
- [GitHub Copilot Provider](42b_GitHub_Copilot_Provider.md)
