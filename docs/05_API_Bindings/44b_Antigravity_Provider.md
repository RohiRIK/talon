# Antigravity Provider (Google)

> **Status:** 🚧 Planned — Phase 1.5 task 1.5.3
> **Category:** API Bindings

---

## 1. Scope

Google Antigravity 2.0 (announced at Google I/O 2026, CLI: `agy`) is Google's agent-first
development platform. It runs on Gemini 3.5 Flash by default and also exposes Claude and
GPT models via Antigravity quota.

Talon uses Google's official OpenAI-compatible Gemini endpoint as the backend, which accepts
Bearer auth with `GEMINI_API_KEY`. The `agy` CLI is shelled out as a fallback token source
if no env var is set.

Auth chain (resolved once at construction):
1. `GEMINI_API_KEY` env var
2. `GOOGLE_API_KEY` env var
3. `agy auth token` CLI subprocess (shelled out; gracefully fails if `agy` not installed)
4. `LlmError::AuthFailed` if all fail

---

## 2. Struct & Auth

```rust
// crates/talon-llm/src/antigravity.rs
// Feature flag: antigravity-provider

pub struct AntigravityProvider {
    client: reqwest::Client,
    token: String,
    model: String,
}

impl AntigravityProvider {
    /// Google's official OpenAI-compatible endpoint for Gemini models.
    const ENDPOINT: &'static str =
        "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions";
    const DEFAULT_MODEL: &'static str = "gemini-3.5-flash";

    pub fn new() -> Result<Self, LlmError>;

    fn resolve_token() -> Result<String, LlmError> {
        // 1. GEMINI_API_KEY env var (trimmed, non-empty)
        // 2. GOOGLE_API_KEY env var (trimmed, non-empty)
        // 3. agy auth token subprocess (fail gracefully if agy not found)
        // 4. LlmError::AuthFailed
    }
}
```

Token sent as `Authorization: Bearer <token>`. The Gemini OpenAI-compatible endpoint
accepts the API key as a Bearer token.

---

## 3. Request Format

Uses the shared `openai_compat` module (same as `GitHubCopilotProvider` and `CodexProvider`).
The `openai_compat.rs` feature gate expands to include `antigravity-provider`.

Available models (via Antigravity quota when using OAuth):

| Model ID | Notes |
|----------|-------|
| `gemini-3.5-flash` | Default — fast, high quality |
| `gemini-3-pro` | Largest Gemini model |
| `gemini-3-flash` | With thinking variants |
| `claude-sonnet-4-6` | Claude via Antigravity quota |
| `claude-opus-4-6` | Claude Opus via Antigravity quota |

> **Note:** Antigravity-quota models (`antigravity-*` prefixed names) require OAuth credentials
> from the `agy` CLI. With `GEMINI_API_KEY` alone, use the standard `gemini-*` model names.

---

## 4. Config

| Setting | Source | Default |
|---------|--------|---------|
| Token | `GEMINI_API_KEY` → `GOOGLE_API_KEY` → `agy auth token` | required |
| Model | `TALON_LLM_MODEL` env var | `gemini-3.5-flash` |
| Endpoint | constant | `https://generativelanguage.googleapis.com/v1beta/openai/chat/completions` |

Feature flag: `features = ["antigravity-provider"]` in `talon-llm/Cargo.toml`.

---

## 5. Acceptance Criteria

- `GEMINI_API_KEY` used when present (trimmed, non-empty)
- `GOOGLE_API_KEY` used as fallback
- `agy auth token` shelled out as last resort; fails gracefully if `agy` not installed
- `LlmError::AuthFailed` when all three sources fail
- Default model is `gemini-3.5-flash`; `TALON_LLM_MODEL` overrides
- `Arc<dyn LlmProvider>` constructible
- No network calls at construction

---

## Related Documents

### Depends On
- [LLM Provider Abstraction](41_LLM_Provider_Abstraction.md)
- [OpenAI-Compatible Client](42a_OpenAI_Compatible_Client.md)

### See Also
- [GitHub Copilot Provider](42b_GitHub_Copilot_Provider.md)
- [Codex Provider](42c_Codex_Provider.md)
