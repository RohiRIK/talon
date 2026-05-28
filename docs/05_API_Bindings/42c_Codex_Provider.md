# Codex Provider (OpenAI)

> **Status:** 🚧 Planned — Phase 1.5 task 1.5.1
> **Category:** API Bindings

---

## 1. Scope

The OpenAI Codex CLI authenticates users via their ChatGPT account or OpenAI API key.
Talon's `CodexProvider` resolves credentials from env vars only (no Codex CLI print command
exists for programmatic token retrieval).

Auth chain (resolved once at construction):
1. `OPENAI_API_KEY` env var
2. `CODEX_ACCESS_TOKEN` env var (ChatGPT Enterprise access token)
3. `LlmError::AuthFailed` if both absent

---

## 2. Struct & Auth

```rust
// crates/talon-llm/src/codex.rs
// Feature flag: codex-provider

pub struct CodexProvider {
    client: reqwest::Client,
    token: String,
    model: String,
}

impl CodexProvider {
    const ENDPOINT: &'static str = "https://api.openai.com/v1/chat/completions";
    const DEFAULT_MODEL: &'static str = "o4-mini";

    pub fn new() -> Result<Self, LlmError>;

    fn resolve_token() -> Result<String, LlmError> {
        // 1. OPENAI_API_KEY env var (trimmed, non-empty)
        // 2. CODEX_ACCESS_TOKEN env var (trimmed, non-empty)
        // 3. LlmError::AuthFailed
    }
}
```

Token sent as `Authorization: Bearer <token>` (same for both API key and access token —
the OpenAI Chat Completions endpoint accepts both).

---

## 3. Request Format

Uses the shared `openai_compat` module (same as `GitHubCopilotProvider`).
The `openai_compat.rs` feature gate expands to include `codex-provider`.

---

## 4. Config

| Setting | Source | Default |
|---------|--------|---------|
| Token | `OPENAI_API_KEY` → `CODEX_ACCESS_TOKEN` | required |
| Model | `TALON_LLM_MODEL` env var | `o4-mini` |
| Endpoint | constant | `https://api.openai.com/v1/chat/completions` |

Feature flag: `features = ["codex-provider"]` in `talon-llm/Cargo.toml`.

---

## 5. Acceptance Criteria

- `OPENAI_API_KEY` used when present (trimmed, non-empty)
- `CODEX_ACCESS_TOKEN` used as fallback
- `LlmError::AuthFailed` when both absent
- Default model is `o4-mini`; `TALON_LLM_MODEL` overrides
- `Arc<dyn LlmProvider>` constructible
- No network calls at construction

---

## Related Documents

### Depends On
- [LLM Provider Abstraction](41_LLM_Provider_Abstraction.md)
- [OpenAI-Compatible Client](42a_OpenAI_Compatible_Client.md)

### See Also
- [GitHub Copilot Provider](42b_GitHub_Copilot_Provider.md)
- [Antigravity Provider](44b_Antigravity_Provider.md)
