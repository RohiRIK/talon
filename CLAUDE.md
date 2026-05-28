# Talon — Claude Code Context

> Single Rust binary. Multi-channel. Persistent cross-project memory. Self-evolving. WASM plugins.
> **Killer differentiator:** Persistent, queryable, cross-project memory in one binary — no cloud, no Python.

---

## Non-Negotiable Behavioral Rules

These apply to every agent, every session, every task — no exceptions.

### 1. Task Completion Protocol (MANDATORY)

After completing **every individual task** in PLAN.md:

- [ ] Mark the checkbox in `PLAN.md`: `- [ ]` → `- [x]`
- [ ] If the task completes a phase exit gate, update the Phase Status table in this file: `⬜` → `✅`
- [ ] If the phase is done, add a completion note in `roadmap.md` under that phase's week

Never batch these updates. Do them the moment a task is done, before starting the next one. The plan is the source of truth for what has been built.

### 2. Prompting Skill (MANDATORY for any LLM prompt)

Before writing any system prompt, agent briefing, LLM instruction, or tool description:

**Invoke:** `Skill("Prompting")` to load the standards.

Then apply:
- **Markdown-first** — no XML tags in prompts; use headers, bullets, tables
- **Briefing primitive** — use `Briefing.hbs` pattern for agent-to-agent handoffs
- **Gate primitive** — use `Gate.hbs` pattern for validation checklists and exit gates
- **Separation of concerns** — structure in template, content in data, logic in code
- **Stable content first** — system prompt and tool definitions before volatile content (user input), so Anthropic's prompt cache prefix stays intact

This applies to: Talon's system prompt, tool `schema()` descriptions, `SessionSearchTool` prompts, `FactExtractor` extraction prompt, `WorkingMemory` summarization prompt, any `AgentEvent` display strings used as LLM input.

---

## Current State

**Phase: 5 — Next.** Phases 0–4 complete (2026-05-28). CLI, TUI (ratatui MVU), HTTP (axum), and Telegram gateways done. `cargo nextest run --workspace` → 349/349 green.
The next action is Phase 5 (Tools Tier 2 + MCP). Task 4.19 (`talon init` onboarding wizard) is deferred — user decision. See `PLAN.md`.

---

## Workspace Layout (locked)

```
talon/                         # workspace root
├── Cargo.toml                 # edition="2024", resolver="2"
├── talon/src/main.rs          # binary crate
└── crates/
    ├── talon-core/            # agent loop, approval membrane
    ├── talon-llm/             # LlmProvider trait + impls
    ├── talon-memory/          # Talon LTM + LanceDB + sessions
    ├── talon-tools/           # all built-in tools
    ├── talon-gateway/         # CLI/TUI, Telegram, Discord, HTTP
    └── talon-plugins/         # WASM host (wasmtime)
```

---

## 7 Load-Bearing Types (locked after Phase 0.5 prototype)

These are defined ONCE in their home crate. Never redefine locally. Never import from the wrong crate.

| # | Type | Location |
|---|------|----------|
| 1 | `ToolResult` struct | `crates/talon-core/src/tools/mod.rs` |
| 2 | `pub trait Tool: Send + Sync` | `crates/talon-core/src/tools/mod.rs` |
| 3 | `pub struct Database` (deadpool-sqlite wrapper) | `crates/talon-memory/src/lib.rs` |
| 4 | `pub trait LlmProvider: Send + Sync` | `crates/talon-llm/src/lib.rs` |
| 5 | `Arc<dyn Tool>` — NOT `Arc<Box<dyn Tool>>` | everywhere |
| 6 | `ApprovalLevel` enum | `crates/talon-core/src/approval.rs` |
| 7 | `AgentEvent` enum | `crates/talon-core/src/events.rs` |

**Type #3 critical:** `rusqlite::Connection` is `!Send`. Never wrap in `tokio::sync::Mutex`. Use `deadpool-sqlite` pool or `tokio::task::spawn_blocking`. Never expose `Connection` across an await point.

---

## Architecture Decisions (already made — do not relitigate)

### Memory Stack

**Decision:** Talon LTM + LanceDB + SQLite + Honker. No Redis.

| Layer | Component | Role |
|-------|-----------|------|
| Memory model | **Talon LTM** (own Rust crate, claude-ltm blueprint) | categories, importance 1–5, decay, FTS5-first search, auto-extraction |
| Memory storage | **LanceDB** (embedded, Apache 2.0) | vectors + FTS + hybrid search, no server required |
| Coordination DB | **SQLite** | sessions, config, messages, Honker queues/cron |
| Reactive layer | **Honker** (`honker-core` crate) | queues, NOTIFY/LISTEN, scheduled maintenance — **optional, add after v1.0** |

There are **two databases, not competing ones**: LanceDB owns *what the agent knows* (memories, facts, embeddings). SQLite owns *how the agent operates* (session state, config, task queues). They do not overlap.

**Redis is not a dependency.** Redis Iris patterns (two-tier memory, fact extraction, semantic dedup, semantic cache) are implemented in pure Rust via Talon LTM + LanceDB. The feature flag `redis-memory` was dropped.

### Error Handling

- `thiserror` in library crates (`talon-core`, `talon-llm`, `talon-memory`, etc.)
- `anyhow` in the binary crate (`talon/`)
- Never mix them in the same crate

### Async

- Rust edition 2024 — native `async fn in trait`. **Never add the `async-trait` crate.**
- If `dyn LlmProvider` object safety is required: return `Pin<Box<dyn Future>>` from the trait method, or use concrete enum dispatch.

### Testing

- **Never `cargo test`** — always `cargo nextest run`
- Mock `LlmProvider` lives in `crates/talon-llm/src/mock.rs` behind `#[cfg(any(test, feature="mock"))]`

### Tool Dispatch

- Sequential dispatch is the **default** (`dispatch_sequential`)
- Parallel dispatch (`dispatch_parallel`, JoinSet + Semaphore cap 4) is **opt-in** via `ToolContext::allow_parallel`

### Terminal Sandbox Backends

Two backends, configured explicitly in `~/.talon/config.toml [tools.terminal] backend`:

| Value | Isolation | `rm -rf /` | Default |
|-------|-----------|------------|---------|
| `"docker"` | Full — seccomp + no network + memory cap | Blocked | ✅ Yes |
| `"native"` | None — runs on host | NOT blocked | ❌ No |

`native` is a legitimate user choice (no Docker, power users, CI runners). It is **never a silent fallback**. Rules:
- `talon init` detects missing Docker → sets `native` explicitly with a printed warning
- `native` always runs at `ApprovalLevel::Dangerous` — every command requires user approval
- Every `native` tool result is prefixed `[NATIVE]` so the LLM and user always know

### TUI Architecture

**Pattern:** MVU (Model-View-Update, Elm-style). All async events → `mpsc` channels → single update loop. Render is pure: `View(Model) → Frame`. No `Mutex` on UI state.

**Stack:** Ratatui (immediate mode) + Crossterm (backend). Reference: OpenCode (Go/Bubbletea) for UX parity.

**Five components:** `ChatView` (streaming markdown) · `InputBar` (`tui-textarea`, history, autocomplete) · `ToolPanel` (collapsible, spinners, diff view) · `StatusBar` (model, tokens, session, `[NATIVE]` badge) · `SplitPane` (adaptive: `<80 cols` stacked, `≥120 cols` side-by-side)

**Three render modes** (detected at startup, overridable by flag):
- `TUI` — full ratatui (default for interactive terminals)
- `Accessible` — line-by-line, no escapes (`--accessible` or `--no-tui`)
- `Plain` — raw text, no colour (`NO_COLOR`, `$TERM=dumb`, piped stdin, CI)

**Markdown:** `comrak` (parse AST) → `syntect` (highlight code blocks) → ratatui `Spans`. Streaming: parse per frame, dim `…` indicator on unclosed blocks.

**Diff rendering:** `similar` crate — red/green unified diff in `ToolPanel` for every `EditFileTool` proposal. User sees the change before it's applied.

**Images:** `ratatui-image` with auto-protocol detection: Kitty → iTerm2 → Sixel → halfblocks (any terminal). Disabled inside tmux/zellij.

**Links:** OSC 8 clickable hyperlinks where terminal supports it.

**Never:** build the TUI without the non-TUI fallback. `Plain` and `Accessible` modes are not optional.

See `docs/10_TUI/` (docs 77–79) for full research.

### Browser Tool

- Use `headless_chrome` crate (actively maintained CDP client), NOT `chromiumoxide` (has axum 0.7+ dep conflicts as of 2025)
- Mark as `feature = "browser"`, experimental

### Semantic Search / Embeddings

- `fastembed` (all-MiniLM-L6-v2, ONNX) behind `feature = "semantic-search"`
- Binary without semantic-search: ~20–30 MB stripped
- Binary with semantic-search: ~50–90 MB stripped (fastembed adds 30–60 MB)
- Never gate CI on binary size; track with `cargo bloat`

### Skill Evolution

- Python sidecar (`uv` managed) with `dspy-ai` / GEPA optimizer
- **v2 feature only.** Not on the v1.0 critical path. Do not build it in Phases 0–5.

---

## Anti-Patterns — Never Do This

- **NEVER** redefine the 7 load-bearing types locally — import from their home crate
- **NEVER** put `rusqlite::Connection` in `tokio::sync::Mutex` — use `spawn_blocking` or `deadpool-sqlite`
- **NEVER** use `std::sync::Mutex` inside async — always `tokio::sync::Mutex` (for `Send` types)
- **NEVER** use `.unwrap()` outside `#[cfg(test)]` — use `?` or `expect("invariant: ...")`
- **NEVER** wrap `Box<dyn Tool>` in `Arc` — use `Arc<dyn Tool>` directly
- **NEVER** mix `anyhow` and `thiserror` in the same crate
- **NEVER** use `cargo test` — use `cargo nextest run`
- **NEVER** add `async-trait` crate — edition 2024 has native async fn in traits
- **NEVER** call `LlmProvider` without a `tokio::time::timeout` wrapper
- **NEVER** spawn a tool without `ApprovalMembrane::check()` first
- **NEVER** log raw LLM prompts at INFO level — DEBUG only (PII risk)
- **NEVER** hold a DB connection across `.await` — open inside `spawn_blocking` closure, close when closure returns
- **NEVER** make `dispatch_parallel` the default — sequential is the safe default; parallel is opt-in
- **NEVER** treat Phase 6 (WASM) or Phase 7 (subagents/evolution) as v1.0 blockers — they are v1.1 and v2

---

## Quality Gates (run after every phase)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used
cargo nextest run --workspace
cargo audit
cargo bloat --release --crates -n 20    # observe only, do not gate on size
docker build -t talon:phase-N .
```

---

## Phase Status

| Phase | Name | Status | Exit Gate |
|-------|------|--------|-----------|
| 0 | Foundation | ✅ Complete (2026-05-27) | `cargo build --workspace --release` green, CI green |
| 0.5 | Working Prototype | ✅ Complete (2026-05-27) | `cargo run -- --message "read Cargo.toml"` works E2E |
| 1 | Core Agent Loop | ✅ Complete (2026-05-27) | Real LLM response, messages persisted to SQLite |
| 1.5 | Additional LLM Providers | ✅ Complete (2026-05-28) | Codex, ClaudeCode, Antigravity + live smoke test, 219 tests green |
| 2 | Memory (FTS5) | ✅ Complete (2026-05-27) | FTS5 search <50ms, context within token budget |
| 2.5 | Talon LTM + LanceDB | ⬜ Not started | Auto fact recall across sessions |
| 3 | Tools Tier 1 | ✅ Complete (2026-05-28) | 275 tests green; fs tools + Docker/native sandbox + TimeoutWrapper |
| 4 | Gateway | ✅ Complete (2026-05-28) | CLI + Telegram + HTTP all functional; 349 tests green |
| 5 | Tools Tier 2 + MCP | ⬜ Not started | MCP server tools discoverable, web search works |
| 6 | Plugins + Scheduling | ⬜ v1.1 | WASM plugin loads without restart |
| 7 | Advanced | ⬜ v2 | Parallel subagents, skill evolution |

**MVP:** Phases 0–2 + 4. Agent that talks, remembers, reachable via Telegram.

---

## Versioning & Release Pipeline

> **Catchphrase:** "Secure by design. TRUST it — it's built on RUST." (TRUST = T + RUST)

- **Version:** single source of truth in `[workspace.package] version` in root `Cargo.toml`
- **Scheme:** semver — start `0.1.0`, bump to `1.0.0` only when all Final Acceptance Criteria pass
- **Tag format:** `v0.1.0` — annotated tags, manual and deliberate. CI never auto-publishes.
- **Changelog:** `git-cliff` + `cliff.toml`. Never hand-edit CHANGELOG.md after Phase 0.
- **Distribution:** GitHub Releases (cargo dist) · crates.io (library crates) · Homebrew · AUR · Docker Hub
- **NOT npm.** Talon is a Rust binary and Rust library crates.

### CI/CD Security (non-negotiable rules)

| Rule | Why |
|------|-----|
| `permissions: {}` at workflow top level | Deny-all default; grant minimum per job |
| All actions pinned to exact SHA | Mutable tags (`@v4`) are a supply-chain attack surface |
| OIDC for crates.io + Docker Hub | No stored API tokens or passwords |
| cosign keyless signing for every binary | Signed via GitHub OIDC, no private key to leak |
| SLSA L2 provenance (`actions/attest-build-provenance`) | Verifiable build integrity |
| `cargo audit` + `cargo deny` on every PR | Block CVEs and license violations before merge |
| Dependabot weekly (Rust + Actions) | SHA bumps handled automatically |

### Workflow Files (planned — created in Phase 0)

| File | Purpose |
|------|---------|
| `.github/workflows/ci.yml` | fmt → clippy → nextest → audit → deny → docker (all PRs + main) |
| `.github/workflows/release.yml` | build → sign → attest → release → crates.io → Docker Hub (tags only) |
| `.github/dependabot.yml` | Weekly Rust + Actions dep bumps |
| `.github/SECURITY.md` | Responsible disclosure policy |
| `.github/CODEOWNERS` | Mandatory review routing |
| `cliff.toml` | git-cliff CHANGELOG config |
| `dist-workspace.toml` | cargo dist release config |
| `deny.toml` | cargo-deny license + advisory rules |
| `lefthook.yml` | Pre-commit: fmt + clippy + nextest |
| `install.sh` | Verifies SHA256 + cosign signature before installing |

---

## Key Files

| File | Purpose |
|------|---------|
| `PLAN.md` | Full implementation plan with per-task detail |
| `roadmap.md` | Chronological timeline, dependency graph, risk register |
| `CHANGELOG.md` | What changed and why |
| `docs/09_Redis_Iris/` | Memory architecture research (Redis Iris → LanceDB decision) |
| `docs/09_Redis_Iris/72_Claude_LTM_Analysis.md` | Blueprint for Talon LTM design |
| `docs/09_Redis_Iris/73_LanceDB_Analysis.md` | Why LanceDB was chosen |
| `docs/09_Redis_Iris/76_Honker_Reactive_Layer.md` | Honker — optional reactive plumbing |

## Knowledge Graph (free — use it)

A full **graphify knowledge graph** of all project documentation is available at no cost:

| Resource | What it contains |
|----------|-----------------|
| `graphify-out/GRAPH_REPORT.md` | Human-readable report — 3130 nodes, 2989 edges, 214 communities. Doc relationships, key concepts, dependency chains between decisions. |
| `graphify-out/graph.json` | Machine-readable graph (nodes + edges + community assignments) |
| `graphify-out/graph.html` | Interactive visual explorer — open in browser |

**When to use it before any phase:**
- Search `GRAPH_REPORT.md` for concepts related to the work — decisions may already be documented
- Check which docs depend on or contradict a design choice before changing it
- Understand blast radius of an architecture change
- Find prior analysis on a topic before starting new research

The graph is regenerated whenever docs update. Read it freely — no API call, no cost.

---

## LLM Provider Notes

- Always wrap calls: `tokio::time::timeout(Duration::from_secs(60), provider.complete(...))`
- Rate limit retries: exponential backoff with jitter, max 3 retries
- Mock provider for tests: deterministic responses, no network

---

## Approved Workspace Dependencies (Phase 0)

```toml
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
futures = "0.3"
deadpool-sqlite = "0.9"
rusqlite = { version = "0.32", features = ["bundled", "vtab"] }
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
axum = "0.7"
wasmtime = "24"                    # Phase 6 — feature-flagged
teloxide = { version = "0.13", features = ["macros"] }
ratatui = "0.28"
crossterm = "0.28"
clap = { version = "4", features = ["derive"] }
thiserror = "1"
anyhow = "1"
lancedb = "0.9"                    # Phase 2.5 — memory storage
arrow-array = "52"                 # LanceDB data
tokio-stream = "0.1"
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }
```

Optional features (not in default workspace, add when phase starts):
```toml
fastembed = "3"                    # feature = "semantic-search"
redis = { version = "0.26", features = ["tokio-comp", "json"] }  # NOT used — decision reversed
headless_chrome = "1"              # feature = "browser"
```
