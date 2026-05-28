# LLM Provider Architecture

> **Status:** ✅ Built (Phase 1.5 complete, 2026-05-28)
> **Category:** Core Feature

---

## 1. Overview

All LLM communication goes through a single trait. Providers are selected at
runtime via `TALON_LLM_PROVIDER`. No provider is hardcoded — swapping one for
another requires only a different env var.

---

## 2. LlmProvider Trait (`crates/talon-llm/src/lib.rs`)

```rust
// Object-safe: returns Pin<Box<dyn Future>> (Rust 2024 — no async-trait crate).
pub trait LlmProvider: Send + Sync {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        tools: &'a [serde_json::Value],
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, LlmError>> + Send + 'a>>;
}
```

The agent always calls through this trait. Provider details are invisible above.

---

## 3. Provider Inventory

| Env value | Struct | Feature flag | Auth mechanism | Notes |
|-----------|--------|-------------|----------------|-------|
| `anthropic` (default) | `AnthropicProvider` | (always on) | `TALON_LLM_API_KEY` or OS keychain | Direct Anthropic API |
| `github-copilot` / `copilot` | `GitHubCopilotProvider` | `github-copilot-provider` | `GITHUB_TOKEN` or `gh auth token` | OpenAI-compat endpoint |
| `openai` | `OpenAIProvider` | `openai-provider` | `OPENAI_API_KEY` | Direct OpenAI API |
| `codex` | `CodexProvider` | `codex-provider` | `gh auth token` | GitHub Copilot Codex API |
| `claude-code` | `ClaudeCodeProvider` | `claude-code-provider` | `claude` CLI on PATH | Shells out to Claude Code CLI |
| `antigravity` | `AntigravityProvider` | `antigravity-provider` | Internal token | Internal provider |

The talon binary enables `github-copilot-provider` by default (its Cargo.toml).
Other providers can be added by enabling their feature flag.

---

## 4. Selection Flow

```
talon --gateway cli
  │
  └── cmd_run()
        │
        ├── read TALON_LLM_PROVIDER (default: "anthropic")
        │
        ├── if provider needs API key: read TALON_LLM_API_KEY / keychain
        │   (github-copilot, claude-code, codex skip this step)
        │
        └── build_gateway_context(provider_name, api_key)
                  │
                  └── match provider_name {
                        "github-copilot" => GitHubCopilotProvider::new()
                        "anthropic" | _  => AnthropicProvider::new(api_key)
                      }
                  └── GatewayContext { provider, tools, db }
```

---

## 5. GitHub Copilot Provider

The most useful provider for development — no separate API key needed if `gh`
is authenticated.

```bash
# Auth check:
gh auth token     # should print a token starting with gho_...

# Run with Copilot:
TALON_LLM_PROVIDER=github-copilot cargo run -- --message "hello"

# Override model:
TALON_LLM_PROVIDER=github-copilot \
TALON_LLM_MODEL=gpt-4o \
cargo run -- --message "hello"
```

Default model: `claude-sonnet-4.6` (served via GitHub Copilot's proxy).

The provider sends to `https://api.githubcopilot.com/chat/completions` using
the OpenAI-compatible request format (`openai_compat.rs`).

---

## 6. OpenAI-Compat Layer (`crates/talon-llm/src/openai_compat.rs`)

GitHub Copilot and OpenAI both use the same request/response format.
Shared logic lives in `openai_compat.rs`:

- `build_body(model, messages, tools)` — serialises to OpenAI chat format
- `check_status(resp)` — maps HTTP errors to `LlmError`
- `parse_response(raw)` — maps OpenAI response to `LlmResponse`

---

## 7. Model Override

Any provider respects `TALON_LLM_MODEL`:

```bash
TALON_LLM_PROVIDER=github-copilot TALON_LLM_MODEL=gpt-4o cargo run -- ...
```

If not set, each provider uses its own `DEFAULT_MODEL` constant.

---

## 8. Mock Provider (tests only)

```rust
// Available with feature = "mock" or in #[cfg(test)]
MockProvider::text("hello", "end_turn")  // returns fixed text
MockProvider::tool_call("read_file", json!({"path": "x"}))  // returns tool call
```

Used in all unit and integration tests. Never makes network calls.

---

## Related Documents

- [Gateway Architecture](../02_Architecture/18_Gateway_MultiChannel_Architecture.md)
- [Core Agent Loop](../02_Architecture/13_Core_Agent_Loop_Design.md)
