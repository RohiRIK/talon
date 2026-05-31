# AGENTS.md — Talon

Architecture reference for AI coding assistants working on this codebase.
Read this before making any significant change.

---

## Rule 1 — Test the Agent via the GitHub CLI (`gh`)

**Live and smoke tests run through the key-less `github-copilot` provider — never ask for or hardcode an API key.** Auth resolves from `GITHUB_TOKEN`, falling back to `gh auth token`. Run `gh auth login` once, then:

```bash
TALON_LLM_PROVIDER=github-copilot cargo run -- --message "remember that I prefer dark mode"
# new session, same ~/.talon/talon.db:
TALON_LLM_PROVIDER=github-copilot cargo run -- --message "what do you know about my preferences?"
```

- Only `anthropic` and `openai` need `TALON_LLM_API_KEY`. `github-copilot` is key-less (`needs_api_key == false` in `talon/src/main.rs`).
- Default model: `claude-sonnet-4-5`. Provider impl: `crates/talon-llm/src/github_copilot.rs`.
- Unit/integration tests still use the deterministic `MockProvider` (no network); `gh` is for end-to-end/live verification only.

---

## What Talon Is

Single Rust binary — AI agent with persistent memory, multi-channel gateways,
and a built-in tool system. No Python. No cloud dependency. Ships as one
self-contained executable.

**Catchphrase:** "TRUST it. It's RUST." (TRUST = T + RUST)

---

## Workspace Layout

```
talon/                          workspace root
├── Cargo.toml                  edition="2024", resolver="2"
├── CLAUDE.md                   full project rules (read this too)
├── PLAN.md                     per-task implementation plan
├── talon/src/main.rs           binary entrypoint
└── crates/
    ├── talon-core/             agent loop, approval membrane, Tool trait
    ├── talon-llm/              LlmProvider trait + all provider impls
    ├── talon-memory/           SQLite sessions + FTS5 search
    ├── talon-tools/            all built-in tools (fs, terminal, search)
    └── talon-gateway/          CLI, TUI, HTTP, Telegram gateways
```

---

## 7 Load-Bearing Types — Never Redefine

These are defined once. Import from their home crate; never duplicate.

| Type | Location |
|------|----------|
| `ToolResult` | `talon-core/src/tools/mod.rs` |
| `Tool` trait | `talon-core/src/tools/mod.rs` |
| `Database` | `talon-memory/src/lib.rs` |
| `LlmProvider` trait | `talon-llm/src/lib.rs` |
| `Arc<dyn Tool>` (NOT `Arc<Box<dyn Tool>>`) | everywhere |
| `ApprovalLevel` | `talon-core/src/approval.rs` |
| `AgentEvent` | `talon-core/src/events.rs` |

---

## How the Agent Loop Works

```
main.rs
  └── cmd_run()
        ├── read TALON_LLM_PROVIDER → build provider
        ├── build_gateway_context(provider, tools, db)
        └── gateway.run()
              └── on each user message:
                    GatewayContext::build_agent(event_tx)
                      └── Agent::run(session_id, text)
                            ├── LlmProvider::complete(messages, tools)
                            ├── if tool call → ApprovalMembrane::check()
                            │     if approved → Tool::execute()
                            └── emit AgentEvent stream → event_tx
```

Each gateway gets a fresh `Agent` per message. No shared mutable agent state.

---

## LLM Providers

Select with `TALON_LLM_PROVIDER` env var:

| Value | Auth needed | Notes |
|-------|-------------|-------|
| `anthropic` (default) | `TALON_LLM_API_KEY` | Direct Anthropic API |
| `github-copilot` | `gh auth token` or `GITHUB_TOKEN` | No separate key needed |
| `openai` | `OPENAI_API_KEY` | |
| `codex` | `gh auth token` | GitHub Copilot Codex |
| `claude-code` | `claude` CLI on PATH | Shells out |

Override model: `TALON_LLM_MODEL=gpt-4o`

For development without an Anthropic key:
```bash
TALON_LLM_PROVIDER=github-copilot cargo run -- --message "hello"
```

---

## Gateways

Select with `--gateway <name>`:

| Flag | Type | Notes |
|------|------|-------|
| `cli` (default) | `CliGateway` | stdin REPL + indicatif spinner |
| `tui` | `TuiGateway` | ratatui MVU; auto-degrades to cli if no TTY |
| `http` | `HttpGateway` | `POST /v1/messages` on 127.0.0.1:7777 |
| `telegram` | `TelegramGateway` | requires `--features talon-gateway/telegram` |

Telegram also requires `TELEGRAM_BOT_TOKEN`.
First sender is auto-registered as owner (`~/.talon/telegram_owner`).
Override with `TELEGRAM_ALLOWED_USER_IDS=<id1,id2>`.

---

## Tools

Built-in tools in `talon-tools`:

| Tool | Approval | What it does |
|------|----------|-------------|
| `echo` | Safe | Returns its input — smoke test tool |
| `read_file` | Safe | Reads a file |
| `write_file` | NeedsApproval | Writes a file |
| `edit_file` | NeedsApproval | Replaces a substring in a file |
| `glob` | Safe | Lists files matching a pattern |
| `grep` | Safe | Searches file content |
| `session_search` | Safe | FTS5 search over conversation history |
| `run_command` | Dangerous | Executes shell command (Docker or native sandbox) |
| `send_message` | NeedsApproval | Agent pushes to a gateway channel |

The approval membrane (`talon-core/src/approval.rs`) gates every tool call.
`Dangerous` tools require explicit user approval. HTTP gateway auto-denies them.

---

## Testing

```bash
# All tests (always use nextest, never cargo test)
cargo nextest run --workspace

# With Telegram feature
cargo nextest run -p talon-gateway --features telegram

# Live smoke tests (require real credentials, skipped by default)
cargo nextest run --run-ignored all -E 'test(smoke)'
```

Mock provider for unit tests:
```rust
MockProvider::text("response text", "end_turn")
```

---

## Hard Rules

- `cargo nextest run` only — never `cargo test`
- No `.unwrap()` outside `#[cfg(test)]`
- No `async-trait` crate — Rust 2024 has native async fn in traits
- `thiserror` in lib crates, `anyhow` in binary only — never mix
- `Arc<dyn Tool>` not `Arc<Box<dyn Tool>>`
- Never hold a DB connection across `.await`
- Never log raw LLM prompts at INFO level (PII risk) — use DEBUG
- Trait methods that must be object-safe: return `Pin<Box<dyn Future>>` not `async fn`

---

## Phase Status

| Phase | Status | What was built |
|-------|--------|----------------|
| 0 — Foundation | ✅ 2026-05-27 | Workspace, CI, Dockerfile |
| 0.5 — Prototype | ✅ 2026-05-27 | E2E agent loop |
| 1 — Core Agent | ✅ 2026-05-27 | Real LLM, SQLite persistence |
| 1.5 — Providers | ✅ 2026-05-28 | 6 LLM providers |
| 2 — Memory | ✅ 2026-05-27 | FTS5 session search |
| 3 — Tools Tier 1 | ✅ 2026-05-28 | fs tools, Docker/native sandbox |
| 4 — Gateway | ✅ 2026-05-28 | CLI, TUI, HTTP, Telegram |
| 5 — Tools Tier 2 | ⬜ next | MCP, web search, browser |
| 6 — Plugins | ⬜ v1.1 | WASM |
| 7 — Advanced | ⬜ v2 | Parallel subagents |

Current test count: **346/346** (+ 66 with telegram feature)
