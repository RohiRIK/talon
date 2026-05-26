# Phase-by-Phase Build Guide

> **Purpose:** Actionable checklist for each migration phase from Doc 21 (Migration Roadmap).
> Derived from graphify community analysis — each phase's docs, dependencies, and exit gates.
> "Exit gate" = what must pass before you move to the next phase.

---

## Overview

```
Phase 0 — Foundation         (Week 1)        ← workspace, Docker, CI
Phase 1 — Core Agent Loop    (Weeks 2–3)     ← loop, LLM, tool trait
Phase 2 — Memory             (Weeks 3–4)     ← SQLite, FTS5, sessions
Phase 3 — Tools Tier 1       (Weeks 4–5)     ← file, terminal, web
Phase 4 — Gateway            (Weeks 5–6)     ← Telegram, HTTP
Phase 5 — Tools Tier 2       (Weeks 6–7)     ← browser, MCP, search
Phase 6 — Plugin & Scheduling (Weeks 7–8)   ← WASM, cron, skills
Phase 7 — Advanced Features  (Weeks 8+)      ← subagents, evolution
```

---

## Phase 0 — Foundation

**Primary docs:** 60, 12, 63, 61, 62, 28

### Checklist

- [ ] Create `talon/` repo with `crates/` workspace layout (Doc 60)
  ```
  talon/
  ├── Cargo.toml           (workspace root)
  ├── crates/
  │   ├── talon-core/
  │   ├── talon-llm/
  │   ├── talon-memory/
  │   ├── talon-tools/
  │   ├── talon-gateway/
  │   └── talon-plugins/
  ├── src/
  │   └── main.rs
  └── Dockerfile
  ```
- [ ] Root `Cargo.toml` defines workspace members (Doc 12)
- [ ] GitHub Actions CI: `cargo check`, `cargo test`, `cargo clippy` (Doc 63)
- [ ] Multi-stage Dockerfile with cargo-chef caching (Doc 62)
- [ ] Seccomp profile for code execution sandbox (Doc 61)
- [ ] Pre-commit hook: `cargo fmt --check` + `cargo clippy -- -D warnings`

### Exit Gate
```bash
cargo build --workspace                        # must compile (empty crates OK)
cargo nextest run --workspace                   # 0 tests, 0 failures
docker build -t talon:dev .                    # must produce an image
```

---

## Phase 1 — Core Agent Loop

**Primary docs:** 54, 14, 41, 42, 44, 13, 20, 50, 51

### Checklist

#### Step 1.1 — Error Types (Doc 54)
- [ ] Define `AgentError`, `LlmError`, `ToolError`, `MemoryError`, `GatewayError` in `talon-core`
- [ ] All use `thiserror::Error`
- [ ] `AgentError` wraps all sub-errors via `#[from]`
- [ ] Three-audience pattern documented in `AgentError::display_for_user()`

#### Step 1.2 — LLM Provider Trait (Doc 41, 42)
- [ ] Define `LlmProvider` trait in `talon-llm`
- [ ] Define `LlmRequest`, `LlmResponse`, `Message`, `ContentBlock` types
- [ ] Implement `OpenAiCompatClient` with `reqwest` (Doc 42)
- [ ] Test: mock HTTP server returns Claude-shaped response, `complete()` parses correctly
- [ ] Implement `AnthropicClient` (Doc 43)
- [ ] Implement SSE parser `SseFrame` (Doc 44)
- [ ] Test: real token stream from `/v1/messages` parses to `LlmChunk` stream

#### Step 1.3 — Tool Trait (Doc 14)
- [ ] Define `Tool` trait in `talon-core`
- [ ] Define `ToolContext`, `ToolResult`, `ApprovalLevel` (exactly as in Doc 05_Canonical_Types)
- [ ] Define `ToolRegistry`
- [ ] Write `EchoTool` (test stub, always returns its input)
- [ ] Test: `ToolRegistry::get("echo")` returns the tool

#### Step 1.4 — Approval Membrane (Doc 17)
- [ ] Define `ApprovalMembrane` struct
- [ ] Implement: `Safe` → proceed, `NeedsApproval` → channel-based request, `Dangerous` → confirm
- [ ] Test: `Safe` tool executes without blocking, `Dangerous` tool blocks until approval channel receives `true`

#### Step 1.5 — Agent Loop (Doc 13)
- [ ] `Agent::run(input: AgentInput) -> Result<(), AgentError>`
- [ ] Inner loop: send to LLM → parse response → dispatch tool calls → loop or stop
- [ ] Broadcast `AgentEvent` on every LLM chunk and tool call
- [ ] Iteration counter: abort after `max_iterations`
- [ ] Test: agent with EchoTool, mock LLM that returns one tool call → agent calls EchoTool → mock LLM returns stop → loop ends

#### Step 1.6 — State Machine (Doc 20)
- [ ] Wrap agent loop in `AgentState` enum
- [ ] Session lifecycle: `Idle → Running → Done`
- [ ] Graceful shutdown on SIGTERM: finish current turn, then exit

### Exit Gate
```bash
cargo nextest run -p talon-core -p talon-llm   # all tests pass
# Manual test: run agent with a real API key, send "hello", get a response
TALON_LLM_API_KEY=sk-... cargo run -- --message "hello"
```

---

## Phase 2 — Memory

**Primary docs:** 55, 15, 35, 57, 66

### Checklist

#### Step 2.1 — SQLite Schema (Doc 55)
- [ ] Create `talon-memory` crate
- [ ] Define `Database` struct with `rusqlite` (feature: `"vtab"`)
- [ ] Schema: `sessions`, `messages`, `fts_messages` (FTS5 virtual table)
- [ ] WAL mode enabled at startup
- [ ] `spawn_blocking` wrapper for all DB calls
- [ ] Test: insert message, FTS5 search returns it

#### Step 2.2 — MemoryStore Trait (Doc 35)
- [ ] Implement `MemoryStore` trait on `Database`
- [ ] `save_message()`, `get_history()`, `search_sessions()`
- [ ] `load_memory_file()` / `save_memory_file()` for `MEMORY.md`
- [ ] Test: round-trip — save 5 messages, get_history returns them in order

#### Step 2.3 — Context Assembly (Doc 15)
- [ ] `ContextBuilder::build()` assembles: MEMORY.md + USER.md + recent messages
- [ ] Context window budget: trim oldest messages when over limit
- [ ] Test: 100 messages, budget=4096 tokens → trimmed to fit

#### Step 2.4 — Session Search Tool (Doc 57)
- [ ] `session_search` tool: calls `MemoryStore::search_sessions()`
- [ ] Returns bookend_start + match window + bookend_end
- [ ] Register in `ToolRegistry`

#### Step 2.5 — User Model (Doc 66)
- [ ] `USER.md` read at context assembly time
- [ ] `memory` tool: updates `MEMORY.md` (append/replace/remove sections)
- [ ] Size guardrail: warn if MEMORY.md > 2,200 chars

### Exit Gate
```bash
cargo nextest run -p talon-memory              # all tests pass
# FTS5 search works
# Context assembly with 100+ messages doesn't overflow
# Agent can call session_search and get results
```

---

## Phase 3 — Tools Tier 1

**Primary docs:** 59, 29, 30, 52, 54

### Checklist

#### Step 3.1 — File System Tools (Doc 59)
- [ ] `ReadFileTool` — `ApprovalLevel::Safe`
- [ ] `WriteFileTool` — `ApprovalLevel::NeedsApproval`
- [ ] `PatchTool` — `ApprovalLevel::NeedsApproval`
- [ ] `SearchFilesTool` — `ApprovalLevel::Safe`
- [ ] Path security: all paths confined to profile dir or explicit allow-list
- [ ] Test: ReadFile on non-existent file returns `ToolResult::err`

#### Step 3.2 — Terminal Tool (Doc 29)
- [ ] `TerminalTool` — `ApprovalLevel::Dangerous`
- [ ] `SandboxBackend` trait: `DirectBackend` + `DockerBackend`
- [ ] `DockerBackend`: spins up ephemeral container, copies seccomp profile
- [ ] Background process management: `process` tool for long-running commands
- [ ] Test: `echo hello` in Docker sandbox returns "hello"

#### Step 3.3 — Tool Execution Engine (Doc 30)
- [ ] `ToolDispatcher::execute_parallel(calls: Vec<ToolCall>)` using `JoinSet`
- [ ] Per-call timeout from `ToolContext`
- [ ] Error isolation: one tool failure doesn't kill other parallel calls
- [ ] Test: 3 parallel EchoTool calls, all return, one with injected error → other 2 still succeed

#### Step 3.4 — Async Tool Execution (Doc 52)
- [ ] `spawn_blocking` bridge for sync tool impls
- [ ] `TimeoutWrapper<T: Tool>` applies `tokio::time::timeout` to all execute calls
- [ ] Test: tool that sleeps 10s + 1s timeout → `ToolError::Timeout(1)`

### Exit Gate
```bash
cargo nextest run -p talon-tools               # all tests pass
# Integration test: agent with file tools + terminal tool
# Agent can read a file, modify it, search it
# Docker sandbox: `rm -rf /` fails (seccomp blocks it)
```

---

## Phase 4 — Gateway

**Primary docs:** 18, 47, 45, 34

### Checklist

#### Step 4.1 — Gateway Trait (Doc 18)
- [ ] Define `Gateway` trait in `talon-gateway`
- [ ] Define `AgentInput`, `AgentOutput`, `DeliveryTarget` types
- [ ] `GatewayRouter`: routes `AgentOutput` to correct gateway by platform

#### Step 4.2 — Message Normalization (Doc 47)
- [ ] `normalize_for_platform(text, platform)` — platform-specific markdown rules
- [ ] `split_message(text, max_chars)` — chunked delivery for long responses
- [ ] Telegram: convert `**bold**` → `*bold*`, convert tables → bullet lists

#### Step 4.3 — Telegram Gateway (Doc 45)
- [ ] `TelegramGateway` implementing `Gateway` using `teloxide`
- [ ] Long polling mode (default)
- [ ] Webhook mode (production)
- [ ] Photo / audio / document attachment handling
- [ ] Test: mock Telegram API server, send a message, receive response

#### Step 4.4 — Send Message Tool (Doc 34)
- [ ] `send_message` tool: calls `GatewayRouter::deliver()`
- [ ] Supports: `target = "telegram"` / `"telegram:#channel"` / `"telegram:chat_id"`
- [ ] `action = "list"` returns available targets

### Exit Gate
```bash
# Real Telegram test: set TELEGRAM_BOT_TOKEN, send a message, get a reply
TELEGRAM_BOT_TOKEN=... cargo run -- --gateway telegram
# Send "hello" in Telegram → agent responds within 5 seconds
```

---

## Phase 5 — Tools Tier 2

**Primary docs:** 32, 33, 36

### Checklist

- [ ] `WebSearchTool` with Brave Search backend (Doc 33)
- [ ] `WebExtractTool` with HTML → markdown conversion
- [ ] Rate limiting + cache layer for web tools
- [ ] `BrowserTool` with `chromiumoxide` (Doc 32)
  - [ ] `BrowserPool` for session reuse
  - [ ] `browser_snapshot` returning accessibility tree
  - [ ] `browser_click`, `browser_type`, `browser_navigate`
- [ ] `McpToolAdapter` bridging MCP protocol to Tool trait (Doc 36)

### Exit Gate
```bash
# Web search returns results from Brave API
# Browser can navigate to a URL and take a screenshot
# MCP: connect to a local filesystem MCP server, list its tools
```

---

## Phase 6 — Plugin & Scheduling

**Primary docs:** 17 (WASM), 38, 39, 37

### Checklist

- [ ] WASM plugin host using `extism` crate (Doc 17)
- [ ] `WasmTool` implementing `Tool` trait
- [ ] Plugin discovery from `~/.talon/plugins/`
- [ ] `SkillStore` with SKILL.md parsing + hot-reload (Doc 38)
- [ ] `CronStore` with SQLite persistence (Doc 37)
- [ ] Scheduler loop using `tokio_cron_scheduler` or custom cron parser
- [ ] `cronjob` LLM tool: create/list/pause/resume/remove jobs
- [ ] Profile isolation for cron jobs (Doc 40)

### Exit Gate
```bash
# WASM: load a sample .wasm plugin, agent calls it
# Skills: create a skill, agent loads it on /skill load
# Cron: schedule a job, it fires at the right time
```

---

## Phase 7 — Advanced Features

**Primary docs:** 19, 53, 39, 56

### Checklist

- [ ] `DelegationEngine` with `JoinSet`-based parallel subagent spawning (Doc 53)
- [ ] `max_spawn_depth` guard (prevent infinite subagent recursion)
- [ ] Toolset filtering for subagents
- [ ] ACP protocol client + server mode (Doc 88)
- [ ] `fastembed-rs` embedding pipeline behind `feature = "semantic-search"` (Doc 56)
- [ ] `sqlite-vec` vector storage
- [ ] Hybrid RRF fusion (FTS5 + embeddings)
- [ ] Self-evolution: GEPA + DSPy pipeline evaluating Talon's skill prompts (Doc 39)

### Exit Gate
```bash
# Delegation: `delegate_task` spawns 3 parallel subagents, all return results
# Semantic search: index 50 sessions, semantic query returns conceptually similar results
# Evolution: evolve_skill.py runs against Talon, produces improved skill variant
```

---

## Cross-Phase Quality Gates

These must pass at the END of every phase:

```bash
# 1. No warnings
cargo clippy --workspace -- -D warnings

# 2. All tests pass
cargo nextest run --workspace

# 3. No type violations in docs (from 05_Canonical_Types.md audit checklist)
grep -rn 'ToolOutput' docs/ | wc -l             # must be 0
grep -rn 'Arc<Box<dyn Tool>>' docs/ | wc -l     # must be 0
grep -rn '~/.ernest\|~/.hermes' docs/ | wc -l   # must be 0

# 4. Binary still under size target
cargo build --release
ls -lh target/release/talon                     # < 50MB target

# 5. Docker build succeeds
docker build -t talon:latest .
```

---

## Risk Checkpoints

| Phase | Highest Risk | Mitigation |
|-------|-------------|------------|
| 0 | cargo-chef layer cache invalidation | Pin Rust toolchain version |
| 1 | LLM provider rate limits during tests | Mock HTTP server for unit tests |
| 2 | SQLite WAL file corruption on crash | Test with simulated power-loss |
| 3 | Docker sandbox escape | Seccomp profile + rootless Docker |
| 4 | Telegram rate limiting | Per-chat message queue with backpressure |
| 5 | Browser pool memory leak | BrowserPool::evict() on idle timeout |
| 6 | WASM plugin ABI mismatch | Extism versioned host exports |
| 7 | Subagent spawn depth unbounded | Hard limit: max_spawn_depth = 3 |

---

*Based on graphify community analysis — migration phase structure from Community 11 (Migration Roadmap & Phases)*
