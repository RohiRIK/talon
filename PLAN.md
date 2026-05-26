# Talon — Zero to Hero Implementation Plan

> **Goal:** The best open-source AI agent in the world.
> Single Rust binary. Multi-channel. Persistent cross-project memory. Self-evolving. WASM plugins.
>
> **Killer differentiator:** Persistent, queryable, cross-project memory in a single binary — no cloud,
> no Python runtime, no venv. Start a session on Telegram, continue it in the CLI, search it from
> Discord. That combination does not exist in any other open-source agent.

---

## Competitive Targets

| Competitor | Their weakness | Talon's answer |
|------------|---------------|----------------|
| Claude Code | Node.js, no persistent cross-project memory, CLI-only | Rust binary, FTS5+semantic memory, Telegram/Discord/CLI |
| Hermes Agent | Python GIL, asyncio+sync chaos, no channels | Pure Tokio, typed, single binary, multi-channel |
| OpenClaw | NestJS bloat, TS overhead, no memory | Axum + Tokio, WASM plugins, queryable session store |
| Aider | Python-only, zero persistent memory, no channels | FTS5 memory, multi-channel, skill evolution |
| Goose | Go, limited tool surface | Richer tools, WASM hot-reload, cross-session search |

> **What NOT to claim as an edge:** raw startup speed. Every LLM call takes 2–10s, making sub-100ms
> process start irrelevant to users. The real story is memory + channels + single binary.

---

## Workspace Layout (locked)

```
talon/                         # workspace root
├── Cargo.toml                 # edition="2024", resolver="2"
├── talon/src/main.rs          # binary crate (anyhow)
└── crates/
    ├── talon-core/            # agent loop, approval membrane
    ├── talon-llm/             # LlmProvider trait + impls
    ├── talon-memory/          # SQLite+FTS5, sessions, skills
    ├── talon-tools/           # all built-in tools
    ├── talon-gateway/         # CLI/TUI, Telegram, Discord, HTTP
    └── talon-plugins/         # WASM host (wasmtime)
```

---

## 7 Load-Bearing Types (locked after Phase 0.5 prototype, NEVER redefined)

| # | Type | Location |
|---|------|----------|
| 1 | `ToolResult` struct | `crates/talon-core/src/tools/mod.rs` |
| 2 | `pub trait Tool: Send + Sync` | `crates/talon-core/src/tools/mod.rs` |
| 3 | `pub struct Database` (wraps `spawn_blocking` channel) | `crates/talon-memory/src/lib.rs` |
| 4 | `pub trait LlmProvider: Send + Sync` | `crates/talon-llm/src/lib.rs` |
| 5 | `Arc<dyn Tool>` (NOT `Arc<Box<dyn Tool>>`) | everywhere |
| 6 | `ApprovalLevel` enum | `crates/talon-core/src/approval.rs` |
| 7 | `AgentEvent` enum | `crates/talon-core/src/events.rs` |

> **Type #3 critical note:** `rusqlite::Connection` is `!Send`. You CANNOT wrap it in
> `tokio::sync::Mutex` — the compiler will reject it. Use `deadpool-sqlite` (connection pool
> on blocking threads) or route all DB calls through `tokio::task::spawn_blocking`. The
> `Database` struct must NEVER expose a `Connection` directly across async boundaries.

---

## Anti-Patterns — Never Do This

- **NEVER** redefine the 7 load-bearing types locally — import from their home crate
- **NEVER** put `rusqlite::Connection` in `tokio::sync::Mutex` — `Connection` is `!Send`; use `spawn_blocking` or `deadpool-sqlite`
- **NEVER** use `std::sync::Mutex` inside async — always `tokio::sync::Mutex` (for `Send` types)
- **NEVER** use `.unwrap()` outside `#[cfg(test)]` — use `?` or `expect("invariant: ...")`
- **NEVER** wrap `Box<dyn Tool>` in `Arc` — use `Arc<dyn Tool>` directly
- **NEVER** mix `anyhow` and `thiserror` in the same crate
- **NEVER** use `cargo test` — use `cargo nextest run`
- **NEVER** use the `async-trait` crate — edition 2024 has native async fn in traits
- **NEVER** call `LlmProvider` without a `tokio::time::timeout` wrapper
- **NEVER** spawn a tool without `ApprovalMembrane::check()` first
- **NEVER** log raw LLM prompts at INFO level — DEBUG only (PII risk)
- **NEVER** hold a DB connection across `.await` — open inside `spawn_blocking` closure, close when closure returns
- **NEVER** make `dispatch_parallel` the default — sequential dispatch is the safe default; parallel is opt-in

---

## Cross-Phase Quality Gates (runs after EVERY phase)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used
cargo nextest run --workspace
cargo audit
cargo bloat --release --crates -n 20    # observe, do not gate on size
docker build -t talon:phase-N .
```

> **Binary size reality check:** wasmtime alone is 15–22 MB stripped. fastembed adds 30–60 MB when
> the `semantic-search` feature is enabled. A realistic stripped binary without semantic-search is
> ~20–30 MB. Do not gate CI on a fixed size limit; instead track with `cargo bloat` and document
> what contributes.

---

## Phase 0 — Foundation (Week 1)

> **Edge:** `curl -fsSL talon.sh/install | sh` drops a pre-built binary in 5s. Claude Code needs
> Node. Hermes needs Python+venv. Talon needs nothing. Single binary, zero dependencies.

### Tasks
- [ ] 0.1 Init workspace `Cargo.toml` — `edition="2024"`, `resolver="2"`, `[workspace.package]`, `[workspace.dependencies]`
- [ ] 0.2 Add all shared deps to `[workspace.dependencies]`: tokio (full), tracing, tracing-subscriber, serde, serde_json, futures, deadpool-sqlite, rusqlite (bundled+vtab), reqwest (rustls-tls), axum, wasmtime, teloxide, ratatui, crossterm, clap (derive). **Do NOT add async-trait — use native async fn in trait (edition 2024).**
- [ ] 0.3 Scaffold crates: `cargo new --lib crates/talon-{core,llm,memory,tools,gateway,plugins}` + `cargo new talon`
- [ ] 0.4 Add `rust-toolchain.toml` pinning stable with `components = ["rustfmt", "clippy"]`
- [ ] 0.5 Install dev tools: `cargo install cargo-nextest cargo-chef cargo-watch cargo-audit cargo-bloat cargo-deny`
- [ ] 0.6 Create `.cargo/config.toml` with aliases: `t = "nextest run"`, `c = "clippy --workspace --all-targets -- -D warnings"`
- [ ] 0.7 Write multi-stage `Dockerfile` with `cargo-chef` layer caching, distroless final stage
- [ ] 0.8 Write `.dockerignore` (target/, .git/, docs/, graphify-out/)
- [ ] 0.9 Write `deny.toml` for `cargo-deny` (licenses + advisories)
- [ ] 0.10 Write `.github/workflows/ci.yml` — matrix (ubuntu-latest / macos-latest / windows-latest): fmt → clippy → nextest → audit → deny → docker build (linux only)
- [ ] 0.11 Write pre-commit config (lefthook or pre-commit): fmt + clippy + nextest on staged
- [ ] 0.12 Boilerplate `talon/src/main.rs` — tracing init, clap CLI skeleton (`--message`, `--config`, `--log-level`, `--gateway`)
- [ ] 0.13 Add `talon init` subcommand — creates `~/.talon/` dir, writes starter `config.toml`, prompts for LLM API key, stores in OS keychain (`keyring` crate)
- [ ] 0.14 Create `docs/ADR/` with: `0001-edition-2024.md`, `0002-thiserror-vs-anyhow.md`, `0003-no-async-trait.md`, `0004-rusqlite-spawn-blocking.md`
- [ ] 0.15 Add `README.md`, `LICENSE` (Apache-2.0 + MIT dual), `CONTRIBUTING.md` — lead with the memory story, not startup speed
- [ ] 0.16 Create `install.sh` (curl install): downloads GitHub release binary, verifies checksum, adds to PATH

### Exit Gate
```bash
cargo build --workspace --release
cargo nextest run --workspace          # 0 tests, 0 failures
cargo clippy --workspace -- -D warnings -D clippy::unwrap_used
docker build -t talon:0 .
# CI green on push for all three OS targets
```

### Risks
- edition 2024 ecosystem compatibility (stable since Feb 2025) → pin toolchain; track crates that lag
- `async fn in trait` object safety — `dyn LlmProvider` with async fn needs `#[async_trait]` shim only for object-safe trait objects; use concrete types or `Box<dyn Future>` return if needed

---

## Phase 0.5 — Working Prototype (end of Week 1)

> **Why this phase exists:** The 3-agent critique unanimously flagged the risk of locking 7 types
> before you've built anything. This phase builds a thin end-to-end agent — no DB, no memory, no
> gateway — just enough to prove the LLM + tool dispatch + approval loop actually works. Only then
> do you lock the 7 types in Phase 1.

### Tasks
- [ ] 0.5.1 Temporary `EchoTool` and `ReadFileTool` stub in `talon/src/main.rs` (not in crates yet)
- [ ] 0.5.2 `AnthropicProvider` quick-and-dirty impl: `reqwest::post`, parse `content[0].text`
- [ ] 0.5.3 Inline agent loop: LLM → if tool_use block → execute → feed result back → loop until stop
- [ ] 0.5.4 Inline `ApprovalLevel` check: `Dangerous` tools print "approve? [y/n]" to stderr
- [ ] 0.5.5 Test: `cargo run -- --message "read ./Cargo.toml and tell me the edition"` — must work end-to-end
- [ ] 0.5.6 Identify any type shape that felt wrong during implementation; record in `docs/ADR/0005-prototype-learnings.md`
- [ ] 0.5.7 Once prototype passes the manual test, promote the 7 types to their final crate homes

### Exit Gate
```bash
TALON_LLM_API_KEY=sk-... cargo run -- --message "read ./Cargo.toml, tell me what edition it uses"
# expects: reads file, correctly reports edition, zero crashes
```

---

## Phase 1 — Core Agent Loop (Weeks 2–3)

> **Edge:** Typed `ApprovalLevel` enforced at the trait boundary. Approval is computed per-invocation
> with actual arguments — not a static flag on the tool definition. This prevents tools from lying
> about their danger level based on what the LLM passes.

### Tasks (Graphify critical path: Doc54 → Doc14 → Doc41 → Doc42)
- [ ] 1.1 `crates/talon-core/src/error.rs` — `CoreError` (thiserror): `LlmError`, `ToolError`, `ApprovalDenied`, `Timeout`, `InvalidState`
- [ ] 1.2 **[TYPE #7]** `crates/talon-core/src/events.rs` — `AgentEvent` enum: `Started`, `LlmRequest`, `LlmResponse`, `ToolCalled`, `ToolResult`, `ApprovalRequested { call_id, tool_name, args, tx: oneshot::Sender<ApprovalDecision> }`, `Completed`, `Failed`
- [ ] 1.3 **[TYPE #6]** `crates/talon-core/src/approval.rs` — `ApprovalLevel { Safe, NeedsApproval, Dangerous }` + `ApprovalMembrane::check(level, &tool_name, &args)`
- [ ] 1.4 **[TYPES #1, #2]** `crates/talon-core/src/tools/mod.rs` — `ToolResult` struct + `Tool` trait:
  ```rust
  pub trait Tool: Send + Sync {
      fn name(&self) -> &str;
      fn schema(&self) -> serde_json::Value;
      fn approval_level(&self, args: &serde_json::Value) -> ApprovalLevel;  // per-invocation
      async fn execute(&self, args: serde_json::Value, ctx: ToolContext) -> ToolResult;
  }
  ```
- [ ] 1.5 **[TYPE #5]** `crates/talon-core/src/tools/dispatcher.rs` — `ToolDispatcher`: `HashMap<String, Arc<dyn Tool>>`, `register`, `dispatch_sequential` (default), `dispatch_parallel` (opt-in, JoinSet + Semaphore)
- [ ] 1.6 `crates/talon-llm/src/error.rs` — `LlmError` (thiserror): `RateLimited`, `InvalidResponse`, `Network`, `AuthFailed`, `ContextTooLong`
- [ ] 1.7 **[TYPE #4]** `crates/talon-llm/src/lib.rs` — `LlmProvider` trait + `Message`, `LlmResponse`, `ToolCall` types
- [ ] 1.8 `crates/talon-llm/src/openai.rs` — `OpenAIProvider` impl, `reqwest` + `tokio::time::timeout(60s, ...)`
- [ ] 1.9 `crates/talon-llm/src/anthropic.rs` — `AnthropicProvider` impl
- [ ] 1.10 `crates/talon-core/src/state.rs` — `AgentState` machine: `Idle → Thinking → CallingTool → AwaitingApproval → Completed | Failed`
- [ ] 1.11 `crates/talon-core/src/agent.rs` — `Agent::run(message)`: LLM → parse tool calls → approval → dispatch (sequential) → loop
- [ ] 1.12 Add `#[tracing::instrument(skip(self))]` selectively to **session boundary fns** (agent start, tool dispatch entry points) — NOT on every hot-path fn (100–500ns overhead per call adds up)
- [ ] 1.13 **[TYPE #3 stub]** `crates/talon-memory/src/lib.rs` — minimal `Database` struct using `deadpool_sqlite::Pool`, WAL mode, sessions+messages tables only (enough for Phase 1 persistence; full schema in Phase 2)
- [ ] 1.14 Wire minimal persistence into `Agent` — save every message to `messages` table via `pool.get().await?.interact(|conn| ...)` pattern
- [ ] 1.15 Wire into `talon/src/main.rs`: build Agent → subscribe AgentEvent → print to stdout
- [ ] 1.16 `crates/talon-llm/src/mock.rs` — mock LlmProvider for deterministic tests (`#[cfg(any(test, feature="mock"))]`)
- [ ] 1.17 Unit tests: approval membrane denies Dangerous, dispatcher routes, state machine transitions, per-invocation approval varies by args

### Exit Gate
```bash
cargo nextest run -p talon-core -p talon-llm
TALON_LLM_API_KEY=sk-... cargo run --release -- --message "hello"
# expects: real LLM response, AgentEvent::Completed in logs, message persisted to DB
sqlite3 ~/.talon/talon.db 'SELECT content FROM messages ORDER BY id DESC LIMIT 1;'
```

### Risks
- `async fn in trait` object safety with `LlmProvider` — if `dyn LlmProvider` is needed, return `Pin<Box<dyn Future>>` from the trait method or use a concrete enum dispatch
- `deadpool-sqlite` interaction closure must not `await` — document this invariant in ADR

---

## Phase 2 — Memory (Weeks 3–4)

> **Edge:** FTS5 full-text search built into the binary (rusqlite bundled). Aider has zero persistent
> memory. Claude Code uses flat files. Talon ships a queryable database — zero install required.
> Cross-project session search with a SQL query.

### Tasks
- [ ] 2.1 `crates/talon-memory/src/error.rs` — `MemoryError` (thiserror)
- [ ] 2.2 `crates/talon-memory/src/schema.sql` — expand schema: `sessions`, `messages`, `tool_calls`, `skills`, `user_facts` + FTS5 virtual table `messages_fts`
- [ ] 2.3 `crates/talon-memory/src/migrations.rs` — embedded migrations via `include_str!`, versioned, run on startup
- [ ] 2.4 **[TYPE #3 final]** Expand `Database` with full `deadpool_sqlite::Pool` API; all DB operations use `.interact(|conn| { ... }).await?` pattern — no `Connection` ever crosses an await point
- [ ] 2.5 `crates/talon-memory/src/store.rs` — `MemoryStore` trait: `save_message`, `search_messages(query, limit)`, `recent_messages(session_id, n)`
- [ ] 2.6 `crates/talon-memory/src/sqlite_store.rs` — impl using FTS5 `MATCH` + `rank`
- [ ] 2.7 `crates/talon-memory/src/context.rs` — `ContextBuilder`: system prompt + USER.md + MEMORY.md + recent N messages + FTS5 retrievals, token budget (hard cap 70% context window)
- [ ] 2.8 `crates/talon-memory/src/files.rs` — `UserMd` / `MemoryMd` loaders from `~/.talon/`
- [ ] 2.9 `crates/talon-tools/src/session_search.rs` — `SessionSearchTool` impl (ApprovalLevel::Safe)
- [ ] 2.10 Integration tests: 100+ messages, FTS5 search <50ms, context stays under budget
- [ ] 2.11 Add `talon db vacuum` + `talon db stats` CLI subcommands

### Exit Gate
```bash
cargo nextest run -p talon-memory
cargo run --release -- --message "what did we talk about yesterday?"
sqlite3 ~/.talon/talon.db 'SELECT count(*) FROM messages_fts;'   # > 0
```

### Risks
- FTS5 not compiled → enable `bundled-full` feature; CI checks with `PRAGMA compile_options`
- WAL files balloon → `PRAGMA wal_autocheckpoint=1000`; add `talon db vacuum` CLI command
- `interact` closure blocks the thread pool — keep DB operations <10ms; no network calls inside

---

## Phase 3 — Tools Tier 1 (Weeks 4–5)

> **Edge:** Docker-sandboxed terminal with seccomp — `rm -rf /` is physically blocked. Aider runs on
> host. Claude Code asks. Talon makes it impossible.

### Tasks
- [ ] 3.1 `crates/talon-tools/src/fs/read.rs` — `ReadFileTool` (Safe), 10MB size limit
- [ ] 3.2 `crates/talon-tools/src/fs/write.rs` — `WriteFileTool` (NeedsApproval), atomic write via temp+rename
- [ ] 3.3 `crates/talon-tools/src/fs/edit.rs` — `EditFileTool` (NeedsApproval), exact-string replace, fails if not unique
- [ ] 3.4 `crates/talon-tools/src/fs/glob.rs` — `GlobTool` (Safe) using `globset`
- [ ] 3.5 `crates/talon-tools/src/fs/grep.rs` — `GrepTool` (Safe) using ripgrep core
- [ ] 3.6 `crates/talon-tools/src/terminal/mod.rs` — `TerminalTool` (Dangerous), `SandboxBackend` trait
- [ ] 3.7 `crates/talon-tools/src/terminal/docker.rs` — `DockerSandbox`: `docker run --rm --network=none --memory=512m --security-opt=seccomp=talon-seccomp.json`
- [ ] 3.8 `crates/talon-tools/src/terminal/seccomp.json` — blocks: mount, ptrace, kexec_load, reboot, raw network
- [ ] 3.9 `Dockerfile.sandbox` — minimal Alpine, no root, no setuid
- [ ] 3.10 `crates/talon-tools/src/timeout.rs` — `TimeoutWrapper<T: Tool>` decorator using `tokio::time::timeout`
- [ ] 3.11 `dispatch_sequential` is default; `dispatch_parallel` uses `JoinSet` + global `Semaphore` (default cap 4), opt-in via `ToolContext::allow_parallel`
- [ ] 3.12 Integration tests: read/write/grep/glob work; `rm -rf /` blocked; timeout kills hung process

### Exit Gate
```bash
cargo nextest run -p talon-tools
cargo run --release -- --message "run 'rm -rf /' in sandbox"
# expects: seccomp blocks it, ToolResult::error returned to LLM
docker images | grep talon-sandbox
```

### Risks
- Docker not on host → fallback `SandboxBackend::Native` with rlimit; warn on startup
- Seccomp on macOS → Docker Desktop handles transparently; document

---

## Phase 4 — Gateway (Weeks 5–6)

> **Edge:** Telegram + CLI + HTTP from one binary, unified session memory. Start in Telegram,
> continue in CLI — same context. Build HTTP gateway first (testable without bot tokens), Telegram
> second, Discord last (serenity has heavy deps; sequence to reduce integration risk).

### Tasks
- [ ] 4.1 `crates/talon-gateway/src/lib.rs` — `Gateway` trait + normalized `Message` struct
- [ ] 4.2 `crates/talon-gateway/src/normalize.rs` — markdown normalization per platform
- [ ] 4.3 `crates/talon-gateway/src/cli.rs` — `CliGateway`: stdin/stdout loop
- [ ] 4.4 `crates/talon-gateway/src/http.rs` — `HttpGateway` (axum): `POST /v1/messages`, SSE stream `GET /v1/stream/:session_id` — **build this first; no bot token required**
- [ ] 4.5 `crates/talon-gateway/src/tui.rs` — `TuiGateway` (ratatui + crossterm): split pane, AgentEvent stream renders as status line
- [ ] 4.6 `crates/talon-gateway/src/telegram.rs` — `TelegramGateway` (teloxide): polling + webhook modes
- [ ] 4.7 `crates/talon-tools/src/send_message.rs` — `SendMessageTool` (NeedsApproval): agent pushes to any channel
- [ ] 4.8 `crates/talon-gateway/src/registry.rs` — `GatewayRegistry`: `HashMap<ChannelId, Arc<dyn Gateway>>`
- [ ] 4.9 Update `talon/src/main.rs`: `--gateway cli,telegram,http` flag; spawn each as `tokio::spawn`
- [ ] 4.10 Integration tests: CLI roundtrip, HTTP POST roundtrip with mock LLM
- [ ] 4.11 Manual test: Telegram bot responds within 5s end-to-end

### Exit Gate
```bash
cargo nextest run -p talon-gateway
cargo run --release -- --gateway cli
curl -X POST http://localhost:7777/v1/messages -d '{"content":"hi"}'   # 200 OK
# Telegram: set TELEGRAM_BOT_TOKEN, send "hello" → response <5s
```

---

## Phase 5 — Tools Tier 2 (Weeks 6–7)

> **Edge:** MCP adapter means every Claude Code tool plugs straight in. Browser via CDP without a
> Node.js bridge. Start with stdio-subprocess plugin protocol before WASM — simpler, immediate value.

### Tasks
- [ ] 5.1 `crates/talon-tools/src/web/search.rs` — `WebSearchTool` (Safe), Brave API + DDG fallback
- [ ] 5.2 `crates/talon-tools/src/web/extract.rs` — `WebExtractTool` (Safe): fetch + readable text
- [ ] 5.3 `crates/talon-tools/src/subprocess_plugin.rs` — **stdio subprocess plugin protocol first**: spawn process, exchange JSON over stdin/stdout, expose as `Arc<dyn Tool>`; this is the entry point for plugins before WASM
- [ ] 5.4 `crates/talon-tools/src/mcp/adapter.rs` — `McpToolAdapter`: exposes MCP server tools as `Arc<dyn Tool>`
- [ ] 5.5 `crates/talon-tools/src/mcp/client.rs` — minimal MCP JSON-RPC client (stdio + HTTP transport)
- [ ] 5.6 `~/.talon/mcp_servers.toml` config format
- [ ] 5.7 `crates/talon-tools/src/web/browser.rs` — `BrowserTool` (NeedsApproval) using `headless_chrome` crate (actively maintained CDP client); **mark as experimental feature flag `feature = "browser"`**
- [ ] 5.8 `crates/talon-tools/src/browser/pool.rs` — `BrowserPool`: reuse headless Chrome instances
- [ ] 5.9 Tool timeouts: web=30s, browser=60s, mcp=30s

> **chromiumoxide note:** As of 2025, chromiumoxide has unresolved dep conflicts with axum 0.7+.
> Use `headless_chrome` crate instead (actively maintained). Reassess chromiumoxide at Phase 5 start.

### Exit Gate
```bash
cargo nextest run -p talon-tools --features integration
cargo run --release -- --message "search Rust async news, summarize top 3"
# MCP: connect to a local filesystem MCP server, list its tools
```

---

## Phase 6 — Plugin & Scheduling (Weeks 7–8)

> **Edge:** Hot-reloadable WASM plugins (any language → `.wasm`). Cron-scheduled LLM agents.
> No restart required. Preceded by stdio subprocess protocol (Phase 5) which validates the plugin
> abstraction before committing to WASM ABI complexity.

### Tasks
- [ ] 6.1 `crates/talon-plugins/src/lib.rs` — `PluginHost` using `wasmtime::Engine` + WASI preview2
- [ ] 6.2 `crates/talon-plugins/src/skill.rs` — `Skill` struct: id, path, wasm_module, manifest (capabilities + approval_level)
- [ ] 6.3 `crates/talon-plugins/src/store.rs` — `SkillStore`: load `.wasm` from `~/.talon/skills/`, hot-reload via `notify`
- [ ] 6.4 `crates/talon-plugins/src/sandbox.rs` — capability gating: WASM only calls host functions declared in manifest
- [ ] 6.5 Each skill becomes `Arc<dyn Tool>` adapter (replaces subprocess adapter from Phase 5 for compiled plugins)
- [ ] 6.6 `crates/talon-memory/src/cron.rs` — `CronStore` table: id, expr, prompt, last_run, next_run
- [ ] 6.7 `crates/talon-core/src/scheduler.rs` — `Scheduler`: tokio interval ticker, polls due jobs, invokes `Agent::run`
- [ ] 6.8 `crates/talon-tools/src/cronjob.rs` — `CronJobTool` (NeedsApproval): create/list/delete cron jobs
- [ ] 6.9 `examples/skills/hello/` — example skill compiling to `.wasm`
- [ ] 6.10 Hot-reload test: drop `.wasm` → appears in tool list within 2s
- [ ] 6.11 Cron test: `*/1 * * * *` job fires on the minute

### Exit Gate
```bash
cargo nextest run -p talon-plugins
cd examples/skills/hello && cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/hello.wasm ~/.talon/skills/
cargo run -- --message "use hello skill"   # loads without restart
```

---

## Phase 7 — Advanced Features (Weeks 8+)

> **Edge:** Parallel subagents + ACP + semantic search + optional skill evolution sidecar.
> The semantic search + cross-channel memory combination alone has no open-source equivalent.
> Self-evolution is a v2 feature — ship it as an optional sidecar, not a v1 requirement.

### Tasks
- [ ] 7.1 `crates/talon-core/src/delegate/mod.rs` — `DelegationEngine`: `JoinSet`-based parallel subagent spawning, `max_spawn_depth = 3` hard limit
- [ ] 7.2 `crates/talon-tools/src/delegate.rs` — `DelegateTaskTool` (NeedsApproval)
- [ ] 7.3 `crates/talon-core/src/acp/` — ACP client + server (JSON-RPC over stdio/websocket)
- [ ] 7.4 `crates/talon-memory/src/embeddings.rs` — `EmbeddingStore` using `fastembed` (all-MiniLM-L6-v2, ONNX, feature-flagged `feature = "semantic-search"`); binary without this feature is ~20–30 MB stripped; with it is 50–90 MB
- [ ] 7.5 SQLite schema: `message_embeddings(message_id, vector BLOB)`
- [ ] 7.6 `crates/talon-memory/src/semantic.rs` — cosine similarity + RRF hybrid with FTS5
- [ ] 7.7 `crates/talon-gateway/src/discord.rs` — `DiscordGateway` (serenity or twilight-rs)
- [ ] 7.8 `evolution/` — Python sidecar (`uv` managed): `dspy-ai`, `evolve_skill.py` (GEPA optimizer) — **optional, v2, not required for v1 release**
- [ ] 7.9 `crates/talon-tools/src/evolve.rs` — `EvolveSkillTool` (Dangerous): spawns Python sidecar, captures output, saves improved skill — **feature-flagged `feature = "evolution"`**
- [ ] 7.10 Release pipeline: GitHub Actions `cargo dist` for linux/macos/windows pre-built binaries, signed with `cargo-sigstore`; `install.sh` checksums verified; Homebrew formula; AUR package
- [ ] 7.11 `talon doctor` subcommand — checks API key, DB integrity, plugin health, network connectivity
- [ ] 7.12 Performance benchmarks: `hyperfine 'target/release/talon --message "hi"'` for agent process start (not LLM round-trip); target: consistent, fast enough to not be noticed

### Exit Gate
```bash
cargo nextest run --workspace
cargo run --release -- --message "research Rust async by delegating 3 subagents"
# 3 subagents spawn, merged result, total < sum of individual times
# semantic search (if feature enabled):
cargo run --release --features semantic-search -- --message "find sessions about Telegram bots"
# Distribution:
cargo dist build --release   # produces tarballs for all targets
```

---

## Final Acceptance Criteria

- [ ] `talon init` completes in <5s, creates `~/.talon/` with valid config
- [ ] `curl -fsSL talon.sh/install | sh` installs a working binary
- [ ] Zero `unwrap_used` / `expect_used` clippy lints in production code
- [ ] `cargo nextest run --workspace` green
- [ ] `cargo audit` + `cargo deny check` clean
- [ ] Docker image <100MB (distroless final stage with full features)
- [ ] All 7 load-bearing types defined exactly once, `rusqlite::Connection` never crosses an await point
- [ ] Telegram + CLI + TUI + HTTP all functional
- [ ] WASM plugin loads without restart
- [ ] Docker sandbox blocks `rm -rf /` (verified in test suite)
- [ ] Parallel delegation spawns 3+ subagents, merged result
- [ ] CI matrix green on linux/macos/windows
- [ ] FTS5 session search returns results across projects

## Beat the Competition

- [ ] **vs Claude Code:** Persistent cross-project FTS5 memory, Telegram + Discord, single pre-built binary via `curl | sh`
- [ ] **vs Hermes:** zero GIL, single binary, unified cross-channel memory, no venv needed
- [ ] **vs OpenClaw:** Rust binary vs NestJS, WASM plugins vs npm, queryable session store vs stateless
- [ ] **vs Aider:** persistent FTS5+semantic memory, multi-channel, skill evolution (v2)
- [ ] **vs Goose:** richer tools (browser, MCP, evolution), WASM hot-reload, cross-channel sessions
