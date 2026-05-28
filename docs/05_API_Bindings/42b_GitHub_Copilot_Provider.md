# GitHub Copilot Provider

> **Status:** ✅ Complete
> **Category:** API Bindings

---

## 1. Scope

GitHub Copilot exposes an OpenAI-compatible Chat Completions endpoint that supports multiple
models (Claude, GPT-4o, Gemini). Users are authenticated via their active GitHub session —
no separate API key required.

Auth chain (resolved once at construction, not on every request):
1. `GITHUB_TOKEN` env var
2. `gh auth token` CLI subprocess (blocking — acceptable at construction time)
3. `LlmError::AuthFailed` if both fail

---

## 2. Struct & Auth

```rust
// crates/talon-llm/src/github_copilot.rs
// Feature flag: github-copilot-provider

pub struct GitHubCopilotProvider {
    client: reqwest::Client,
    token: String,
    model: String,
}

impl GitHubCopilotProvider {
    const ENDPOINT: &'static str = "https://api.githubcopilot.com/chat/completions";
    const DEFAULT_MODEL: &'static str = "claude-sonnet-4-5";

    pub fn new() -> Result<Self, LlmError>;

    fn resolve_token() -> Result<String, LlmError> {
        // 1. GITHUB_TOKEN env var (trimmed, non-empty)
        // 2. gh auth token subprocess
    }
}
```

The token is sent as `Authorization: Bearer <token>`.
Additional headers: `editor-version: talon/0.2.0`, `editor-plugin-version: talon-llm/0.2.0`.

---

## 3. Request Format

Uses the shared `openai_compat` module (`build_body`, `check_status`, `parse_response`).
Response parsing is OpenAI Chat Completions format (`choices[0].message`).

---

## 4. Config

| Setting | Source | Default |
|---------|--------|---------|
| Token | `GITHUB_TOKEN` → `gh auth token` | required |
| Model | `TALON_LLM_MODEL` env var | `claude-sonnet-4-5` |
| Endpoint | constant | `https://api.githubcopilot.com/chat/completions` |

Feature flag: `features = ["github-copilot-provider"]` in `talon-llm/Cargo.toml`.

---

## 5. Acceptance Criteria

- `GITHUB_TOKEN` env var is used when non-empty (whitespace trimmed)
- `gh auth token` is shelled out when env var absent or empty
- `LlmError::AuthFailed` returned if both sources fail
- Token resolved at construction, not per-call
- `Arc<dyn LlmProvider>` is constructible

---

## Related Documents

### Depends On
- [LLM Provider Abstraction](41_LLM_Provider_Abstraction.md)
- [OpenAI-Compatible Client](42a_OpenAI_Compatible_Client.md)

### See Also
- [Codex Provider](42c_Codex_Provider.md)
- [Antigravity Provider](44b_Antigravity_Provider.md)
