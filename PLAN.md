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

## Versioning Strategy (locked from Phase 0)

> **Catchphrase:** "Secure by design. TRUST it — it's built on RUST."
> (TRUST = T + RUST. The wordplay is the brand. Security is not an afterthought.)

### Version Scheme

- **Semver** (`MAJOR.MINOR.PATCH`) for all crates and the binary
- All workspace crates share one version via `[workspace.package] version = "..."` in root `Cargo.toml`
- Start at `0.1.0` on Phase 0 completion; `1.0.0` only when Final Acceptance Criteria are all green
- Version is the **single source of truth** — do not hardcode it anywhere else

### Tagging & Release Triggers

- Tags format: `v0.1.0`, `v0.2.0`, etc. — annotated tags (`git tag -a`)
- Releases are **manual and deliberate**: push a `v*` tag → release workflow fires
- No automatic releases on push to `main` — CI only, no publish
- Release branch: `main` only — no separate release branches

### Changelog

- `git-cliff` for automated CHANGELOG.md generation from conventional commits
- Config: `cliff.toml` at workspace root — scopes map to crate names
- CHANGELOG.md is committed; never hand-edited after Phase 0

### Distribution Channels

| Channel | What ships | Tool |
|---------|-----------|------|
| GitHub Releases | Pre-built binaries for all targets | `cargo dist` |
| crates.io | Library crates only (`talon-core`, `talon-llm`, `talon-memory`, `talon-tools`) | `cargo publish` |
| Homebrew | macOS/Linux tap formula | `cargo dist` generates |
| AUR | Arch Linux | Manual PKGBUILD |
| Docker Hub | OCI image | `docker buildx` + `docker push` |

> **NOT npm.** Talon is a Rust binary and library. npm is not a distribution target.

### CI/CD Security Principles (non-negotiable)

1. **Deny-all permissions at workflow level** — `permissions: {}` at the top of every workflow; grant minimum required per-job only
2. **Pin every action to its exact SHA** — never use mutable tags like `@v4` or `@main`; pin by commit SHA and keep the tag as a comment
3. **OIDC for all publishing** — crates.io trusted publishing (no stored API tokens); Docker Hub OIDC; keyless binary signing via sigstore/cosign
4. **Signed releases** — every binary signed with `cosign` (keyless, via GitHub OIDC); checksum file (`SHA256SUMS`) published with every release
5. **SLSA provenance** — `actions/attest-build-provenance` generates L2 provenance for every release artifact
6. **Supply chain gates** — `cargo audit` (CVEs) + `cargo deny` (licenses + banned crates) run on every PR; either fails → PR blocked
7. **Dependabot** — automated PRs for Rust deps (`cargo`) and GitHub Actions (SHA bumps) weekly
8. **SECURITY.md** — responsible disclosure policy in `.github/SECURITY.md` from day one
9. **CODEOWNERS** — every path has an owner; no merge without review from the right person
10. **No secrets in env** — secrets accessed via `${{ secrets.NAME }}` only; never echoed; never in logs

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
- **NEVER** silently fall back to native terminal execution — if Docker is unavailable, `talon init` sets `backend = "native"` explicitly with a warning; the user always knows which mode is active
- **NEVER** allow `native` backend without `ApprovalLevel::Dangerous` on every command — no exceptions

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

**Workspace scaffold**
- [x] 0.1 Init workspace `Cargo.toml` — `edition="2024"`, `resolver="2"`, `[workspace.package]` with `version = "0.1.0"`, `authors`, `license = "MIT OR Apache-2.0"`, `repository`; `[workspace.dependencies]`
- [x] 0.2 Add all shared deps to `[workspace.dependencies]`: tokio (full), tracing, tracing-subscriber, serde, serde_json, futures, deadpool-sqlite, rusqlite (bundled+vtab), reqwest (rustls-tls), axum, wasmtime, teloxide, ratatui, crossterm, clap (derive). **Do NOT add async-trait — edition 2024 native async fn in traits.**
- [x] 0.3 Scaffold crates: `cargo new --lib crates/talon-{core,llm,memory,tools,gateway,plugins}` + `cargo new talon`
- [x] 0.4 Add `rust-toolchain.toml` pinning stable with `components = ["rustfmt", "clippy"]`
- [x] 0.5 Install dev tools: `cargo install cargo-nextest cargo-chef cargo-watch cargo-audit cargo-bloat cargo-deny git-cliff cargo-dist`
- [x] 0.6 Create `.cargo/config.toml` with aliases: `t = "nextest run"`, `c = "clippy --workspace --all-targets -- -D warnings"`
- [x] 0.7 Write multi-stage `Dockerfile` with `cargo-chef` layer caching, distroless final stage
- [x] 0.8 Write `.dockerignore` (target/, .git/, docs/, graphify-out/)

**Supply-chain security**
- [x] 0.9 Write `deny.toml` for `cargo-deny`: allowed licenses (MIT, Apache-2.0, ISC, BSD-2-Clause, BSD-3-Clause, Zlib), deny `unmaintained` and `unsound` advisories, duplicate crate detection
- [x] 0.10 Write `.github/dependabot.yml` — weekly Rust (`cargo`) and GitHub Actions dep bumps; auto-assign to a dedicated `deps` label
- [x] 0.11 Write `.github/SECURITY.md` — responsible disclosure policy: contact email, expected response SLA (48h), embargo window (90 days), CVE process

**CI workflow** (`.github/workflows/ci.yml`)
- [x] 0.12 Top-level: `permissions: {}` (deny all); `concurrency` block (cancel in-progress on same ref)
- [x] 0.13 Jobs — each with `permissions: contents: read` only:
  - `fmt`: `cargo fmt --all -- --check`
  - `clippy`: `cargo clippy --workspace --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used`
  - `test`: `cargo nextest run --workspace` — matrix: ubuntu / macos / windows
  - `build`: `cargo build --workspace --release` — same matrix
  - `audit`: `cargo audit` then `cargo deny check`
  - `docker`: `docker build` (linux only, no push)
- [x] 0.14 Pin **every** action to its exact commit SHA; keep the version tag as a comment (`# v4.1.1`). Never use mutable tags.

**Release workflow** (`.github/workflows/release.yml`)
- [x] 0.15 Trigger: `push: tags: ['v[0-9]+.[0-9]+.[0-9]+']` only — not on branch push
- [x] 0.16 Top-level `permissions: {}`. Per-job grants:
  - build job: `contents: read`
  - sign + attest job: `contents: write`, `id-token: write`, `attestations: write`
  - publish job: `contents: read`, `id-token: write`
- [x] 0.17 Build job: `cargo dist build --release` for all targets (linux x86_64/aarch64, macos x86_64/aarch64, windows x86_64); upload artifacts
- [x] 0.18 Sign + attest job:
  - Install `cosign` (keyless, via GitHub OIDC — no private key stored anywhere)
  - Sign each binary: `cosign sign-blob --yes --oidc-issuer=https://token.actions.githubusercontent.com`
  - Generate `SHA256SUMS` and sign it
  - SLSA L2 provenance: `actions/attest-build-provenance` for every artifact
- [x] 0.19 GitHub Release job: create release from tag, upload binaries + signatures + `SHA256SUMS` + provenance; auto-generate release notes from `git-cliff`
- [x] 0.20 Publish job (crates.io trusted publishing — no stored token):
  - Configure crates.io trusted publisher for this repo in crates.io dashboard
  - Publish library crates in dependency order: `talon-core` → `talon-llm` → `talon-memory` → `talon-tools` → `talon-gateway` → `talon-plugins`
  - `cargo publish --no-verify` (already tested in CI)
- [x] 0.21 Docker publish job: `docker buildx build --platform linux/amd64,linux/arm64`, push to Docker Hub using OIDC; sign image with `cosign`

**Versioning tooling**
- [x] 0.22 Write `cliff.toml` — scopes: `core`, `llm`, `memory`, `tools`, `gateway`, `plugins`, `ci`, `release`; tag pattern `v[0-9]+\.[0-9]+\.[0-9]+`
- [x] 0.23 Write `dist-workspace.toml` — `cargo dist` config: targets, installers (shell script, Homebrew), checksum (sha256), GitHub CI integration
- [x] 0.24 Write `install.sh` — verifies SHA256 checksum and cosign signature before installing; prints "TRUST it. It's RUST." on success

**Code ownership + repo hygiene**
- [x] 0.25 Write `.github/CODEOWNERS` — `* @<owner>` default; crate-level ownership as team grows
- [x] 0.26 Write pre-commit config (`lefthook.yml`): fmt + clippy + nextest on staged files
- [x] 0.27 Boilerplate `talon/src/main.rs` — tracing init, clap CLI skeleton (`--message`, `--config`, `--log-level`, `--gateway`)
- [x] 0.28 Add `talon init` subcommand — creates `~/.talon/` dir, writes starter `config.toml`, prompts for LLM API key, stores in OS keychain (`keyring` crate)
- [x] 0.29 Create `docs/ADR/` — `0001-edition-2024.md`, `0002-thiserror-vs-anyhow.md`, `0003-no-async-trait.md`, `0004-rusqlite-spawn-blocking.md`, `0005-lancedb-memory-backend.md`, `0006-semver-release-pipeline.md`
- [x] 0.30 Add `README.md` — lead with the memory story ("TRUST it — it's built on RUST"), installation (`curl | sh`), quick start; `LICENSE` (Apache-2.0 + MIT dual); `CONTRIBUTING.md`

### Exit Gate
```bash
cargo build --workspace --release
cargo nextest run --workspace          # 0 tests, 0 failures
cargo clippy --workspace -- -D warnings -D clippy::unwrap_used
cargo audit && cargo deny check
docker build -t talon:0 .
# CI green on all three OS targets
# Release workflow dry-run: push a v0.0.1-test tag, verify signing + attestation, delete tag
```

### Risks
- edition 2024 ecosystem compatibility (stable since Feb 2025) → pin toolchain; track crates that lag
- `async fn in trait` object safety — use concrete types or `Pin<Box<dyn Future>>` if `dyn LlmProvider` is needed
- crates.io trusted publishing requires dashboard configuration before first publish — do the one-time setup during Phase 0, not at release time
- `cargo dist` config can require iteration — test with a `v0.0.1-test` tag before the real `v0.1.0`

---

## Phase 0.5 — Working Prototype (end of Week 1)

> **Why this phase exists:** The 3-agent critique unanimously flagged the risk of locking 7 types
> before you've built anything. This phase builds a thin end-to-end agent — no DB, no memory, no
> gateway — just enough to prove the LLM + tool dispatch + approval loop actually works. Only then
> do you lock the 7 types in Phase 1.

### Tasks
- [x] 0.5.1 Temporary `EchoTool` and `ReadFileTool` stub in `talon/src/main.rs` (not in crates yet)
- [x] 0.5.2 `AnthropicProvider` quick-and-dirty impl: `reqwest::post`, parse `content[0].text`
- [x] 0.5.3 Inline agent loop: LLM → if tool_use block → execute → feed result back → loop until stop
- [x] 0.5.4 Inline `ApprovalLevel` check: `Dangerous` tools print "approve? [y/n]" to stderr
- [x] 0.5.5 Test: `cargo run -- --message "read ./Cargo.toml and tell me the edition"` — must work end-to-end
- [x] 0.5.6 Identify any type shape that felt wrong during implementation; record in `docs/ADR/0007-prototype-learnings.md`
- [x] 0.5.7 Once prototype passes the manual test, promote the 7 types to their final crate homes

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
- [x] 1.1 `crates/talon-core/src/error.rs` — `CoreError` (thiserror): `LlmError`, `ToolError`, `ApprovalDenied`, `Timeout`, `InvalidState`
- [x] 1.2 **[TYPE #7]** `crates/talon-core/src/events.rs` — `AgentEvent` enum: `Started`, `LlmRequest`, `LlmResponse`, `ToolCalled`, `ToolResult`, `ApprovalRequested { call_id, tool_name, args, tx: oneshot::Sender<ApprovalDecision> }`, `Completed`, `Failed`
- [x] 1.3 **[TYPE #6]** `crates/talon-core/src/approval.rs` — `ApprovalLevel { Safe, NeedsApproval, Dangerous }` + `ApprovalMembrane::check(level, &tool_name, &args)`
- [x] 1.4 **[TYPES #1, #2]** `crates/talon-core/src/tools/mod.rs` — `ToolResult` struct + `Tool` trait:
  ```rust
  pub trait Tool: Send + Sync {
      fn name(&self) -> &str;
      fn schema(&self) -> serde_json::Value;
      fn approval_level(&self, args: &serde_json::Value) -> ApprovalLevel;  // per-invocation
      async fn execute(&self, args: serde_json::Value, ctx: ToolContext) -> ToolResult;
  }
  ```
- [x] 1.5 **[TYPE #5]** `crates/talon-core/src/tools/dispatcher.rs` — `ToolDispatcher`: `HashMap<String, Arc<dyn Tool>>`, `register`, `dispatch_sequential` (default), `dispatch_parallel` (opt-in, JoinSet + Semaphore)
- [x] 1.6 `crates/talon-llm/src/error.rs` — `LlmError` (thiserror): `RateLimited`, `InvalidResponse`, `Network`, `AuthFailed`, `ContextTooLong`
- [x] 1.7 **[TYPE #4]** `crates/talon-llm/src/lib.rs` — `LlmProvider` trait + `Message`, `LlmResponse`, `ToolCall` types
- [x] 1.8 `crates/talon-llm/src/openai.rs` — `OpenAIProvider` impl, `reqwest` + `tokio::time::timeout(60s, ...)`
- [x] 1.9 `crates/talon-llm/src/anthropic.rs` — `AnthropicProvider` impl
- [x] 1.10 `crates/talon-core/src/state.rs` — `AgentState` machine: `Idle → Thinking → CallingTool → AwaitingApproval → Completed | Failed`
- [x] 1.11 `crates/talon-core/src/agent.rs` — `Agent::run(message)`: LLM → parse tool calls → approval → dispatch (sequential) → loop
- [x] 1.12 Add `#[tracing::instrument(skip(self))]` selectively to **session boundary fns** (agent start, tool dispatch entry points) — NOT on every hot-path fn (100–500ns overhead per call adds up)
- [x] 1.13 **[TYPE #3 stub]** `crates/talon-memory/src/lib.rs` — minimal `Database` struct using `deadpool_sqlite::Pool`, WAL mode, sessions+messages tables only (enough for Phase 1 persistence; full schema in Phase 2)
- [x] 1.14 Wire minimal persistence into `Agent` — save every message to `messages` table via `pool.get().await?.interact(|conn| ...)` pattern
- [x] 1.15 Wire into `talon/src/main.rs`: build Agent → subscribe AgentEvent → print to stdout
- [x] 1.16 `crates/talon-llm/src/mock.rs` — mock LlmProvider for deterministic tests (`#[cfg(any(test, feature="mock"))]`)
- [x] 1.17 Unit tests: approval membrane denies Dangerous, dispatcher routes, state machine transitions, per-invocation approval varies by args

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

## Phase 1.5 — Additional LLM Providers

> **Why:** Real-world agent users authenticate via their active CLI sessions (GitHub Copilot, Gemini,
> Claude Code) rather than raw API keys. Each provider follows the same token-resolver pattern:
> env var → CLI fallback → `LlmError::AuthFailed`. OpenAI-compatible providers share
> `openai_compat.rs`; Anthropic-format providers inline the Messages API format.

### Tasks
- [x] 1.5.0 `crates/talon-llm/src/github_copilot.rs` + `openai_compat.rs` — `GitHubCopilotProvider`: auth `GITHUB_TOKEN` → `gh auth token` CLI; endpoint `https://api.githubcopilot.com/chat/completions`; default model `claude-sonnet-4.6`; feature `github-copilot-provider`
- [x] 1.5.1 `crates/talon-llm/src/codex.rs` — `CodexProvider`: auth `OPENAI_API_KEY` → `CODEX_ACCESS_TOKEN`; endpoint `https://api.openai.com/v1/chat/completions`; default model `o4-mini`; feature `codex-provider`; expand `openai_compat.rs` gate
- [x] 1.5.2 `crates/talon-llm/src/claude_code.rs` — `ClaudeCodeProvider`: auth `CLAUDE_CODE_OAUTH_TOKEN` (Bearer) → `ANTHROPIC_AUTH_TOKEN` (Bearer) → `ANTHROPIC_API_KEY` (x-api-key) → `claude setup-token` CLI; endpoint `https://api.anthropic.com/v1/messages`; default model `claude-opus-4-7`; feature `claude-code-provider`
- [x] 1.5.3 `crates/talon-llm/src/antigravity.rs` — `AntigravityProvider`: auth `GEMINI_API_KEY` → `GOOGLE_API_KEY` → `agy auth token` CLI; endpoint `https://generativelanguage.googleapis.com/v1beta/openai/chat/completions`; default model `gemini-3.5-flash`; feature `antigravity-provider`; expand `openai_compat.rs` gate
- [x] 1.5.4 Unit tests for 1.5.1–1.5.3: token resolution, env override, model default, Arc<dyn LlmProvider> constructible, empty-env rejection (5–6 tests per provider)
- [x] 1.5.5 Live smoke test for `GitHubCopilotProvider` — `#[ignore]` async test fires real Copilot API call, asserts non-empty `Text` block in response; run with `TALON_LLM_MODEL=claude-sonnet-4.5 cargo nextest run --run-ignored all -E 'test(smoke)'`; confirmed passing (3s round-trip)

### Exit Gate
```bash
cargo clippy --workspace --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used
cargo nextest run -p talon-llm
# with each feature: --features talon-llm/codex-provider, claude-code-provider, antigravity-provider
```

---

## Maintenance — Security & CI (2026-05-28)

- [x] M.1 Fix `cargo fmt` drift in `talon-core/src/agent.rs` — CI was failing the format check gate
- [x] M.2 Upgrade `wasmtime 24 → 43` — fixed 6 CVEs (RUSTSEC-2026-0086/0088/0089/0094/0095/0096); crate was a Phase 6 placeholder with no source usage, zero code changes required
- [x] M.3 Upgrade `lancedb 0.9 → 0.29` + `arrow-array 52 → 54` — fixed 3 CVEs in transitive `rustls-webpki 0.101.7` (RUSTSEC-2026-0098/0099/0104) pulled in via old AWS SDK; crate was a Phase 2.5 placeholder with no source usage, zero code changes required
- [x] M.4 CI green on main: fmt ✅ clippy ✅ nextest 219/219 ✅ audit ✅ deny ✅
- [ ] M.5 Re-enable macOS + Windows CI matrix (`os: [ubuntu-latest, macos-latest, windows-latest]`) — dropped to stay within free GitHub Actions minutes during development. Restore before v1.0 release to verify cross-platform builds. See `.github/workflows/ci.yml` test + build jobs.

---

## Phase 2 — Memory (Weeks 3–4)

> **Edge:** FTS5 full-text search built into the binary (rusqlite bundled). Aider has zero persistent
> memory. Claude Code uses flat files. Talon ships a queryable database — zero install required.
> Cross-project session search with a SQL query.

### Tasks
- [x] 2.1 `crates/talon-memory/src/error.rs` — `MemoryError` (thiserror)
- [x] 2.2 `crates/talon-memory/src/schema.sql` — expand schema: `sessions`, `messages`, `tool_calls`, `skills`, `user_facts` + FTS5 virtual table `messages_fts`
- [x] 2.3 `crates/talon-memory/src/migrations.rs` — embedded migrations via `include_str!`, versioned, run on startup
- [x] 2.4 **[TYPE #3 final]** Expand `Database` with full `deadpool_sqlite::Pool` API; all DB operations use `.interact(|conn| { ... }).await?` pattern — no `Connection` ever crosses an await point
- [x] 2.5 `crates/talon-memory/src/store.rs` — `MemoryStore` trait: `save_message`, `search_messages(query, limit)`, `recent_messages(session_id, n)`
- [x] 2.6 `crates/talon-memory/src/sqlite_store.rs` — impl using FTS5 `MATCH` + `rank`
- [x] 2.7 `crates/talon-memory/src/context.rs` — `ContextBuilder`: system prompt + USER.md + MEMORY.md + recent N messages + FTS5 retrievals, token budget (hard cap 70% context window)
- [x] 2.8 `crates/talon-memory/src/files.rs` — `UserMd` / `MemoryMd` loaders from `~/.talon/`
- [x] 2.9 `crates/talon-tools/src/session_search.rs` — `SessionSearchTool` impl (ApprovalLevel::Safe)
- [x] 2.10 Integration tests: 100+ messages, FTS5 search <50ms, context stays under budget
- [x] 2.11 Add `talon db vacuum` + `talon db stats` CLI subcommands

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
> Users who don't want Docker can opt into native host execution — but it is an explicit,
> acknowledged choice with a hard warning, never a silent fallback.

### Terminal Sandbox Design

Two backends, one trait. The backend is set in `~/.talon/config.toml` — never auto-detected silently.

```toml
[tools.terminal]
# "docker"  — sandboxed (default, recommended). Requires Docker on host.
#             rm -rf / is physically blocked via seccomp + network isolation.
# "native"  — runs directly on your machine. No isolation. You are responsible.
#             Talon will warn loudly and require explicit acknowledgement on first use.
backend = "docker"
```

| Backend | Isolation | `rm -rf /` | Who it's for |
|---------|-----------|------------|--------------|
| `docker` | Full (seccomp + no network + memory cap) | Blocked | Default — everyone |
| `native` | None | **NOT blocked** | Power users who explicitly opt in |

**`native` mode behaviour:**
- On first run: prints a one-time warning to stderr and requires `y` confirmation (stored in config so it doesn't repeat every time, but is shown again after any config reset)
- `ApprovalLevel` for `TerminalTool` escalates to `Dangerous` in native mode regardless of the command — every shell execution requires user approval
- A `[NATIVE]` tag is prepended to every tool result so the LLM and user always know which mode is active

### Tasks
- [x] 3.1 `crates/talon-tools/src/fs/read.rs` — `ReadFileTool` (Safe), 10MB size limit
- [x] 3.2 `crates/talon-tools/src/fs/write.rs` — `WriteFileTool` (NeedsApproval), atomic write via temp+rename
- [x] 3.3 `crates/talon-tools/src/fs/edit.rs` — `EditFileTool` (NeedsApproval), exact-string replace, fails if not unique
- [x] 3.4 `crates/talon-tools/src/fs/glob.rs` — `GlobTool` (Safe) using `globset`
- [x] 3.5 `crates/talon-tools/src/fs/grep.rs` — `GrepTool` (Safe) using ripgrep core
- [x] 3.6 `crates/talon-tools/src/terminal/mod.rs` — `TerminalTool` (Dangerous), `SandboxBackend` trait:
  ```rust
  pub trait SandboxBackend: Send + Sync {
      fn mode(&self) -> SandboxMode;  // Docker | Native
      async fn execute(&self, cmd: &str, ctx: &ToolContext) -> ToolResult;
  }
  ```
- [x] 3.7 `crates/talon-tools/src/terminal/docker.rs` — `DockerSandbox`: `docker run --rm --network=none --memory=512m --security-opt=seccomp=talon-seccomp.json`
- [x] 3.8 `crates/talon-tools/src/terminal/native.rs` — `NativeBackend`: runs on host via `tokio::process::Command`; always `ApprovalLevel::Dangerous`; prepends `[NATIVE]` tag; one-time stderr warning + `y` acknowledgement stored in config
- [x] 3.9 `crates/talon-tools/src/terminal/seccomp.json` — blocks: mount, ptrace, kexec_load, reboot, raw network
- [x] 3.10 `Dockerfile.sandbox` — minimal Alpine, no root, no setuid
- [x] 3.11 `crates/talon-tools/src/timeout.rs` — `TimeoutWrapper<T: Tool>` decorator using `tokio::time::timeout`
- [x] 3.12 `dispatch_sequential` is default; `dispatch_parallel` uses `JoinSet` + global `Semaphore` (default cap 4), opt-in via `ToolContext::allow_parallel`
- [x] 3.13 Integration tests: read/write/grep/glob work; `rm -rf /` blocked in Docker mode; native mode warns and tags output; timeout kills hung process

### Exit Gate
```bash
cargo nextest run -p talon-tools
# Docker mode: rm -rf / is blocked
cargo run --release -- --message "run 'rm -rf /' in sandbox"
# Native mode: warning shown, [NATIVE] tag in output, ApprovalLevel::Dangerous enforced
TALON_TERMINAL_BACKEND=native cargo run --release -- --message "run 'echo hello'"
docker images | grep talon-sandbox
```

### Risks
- Docker not installed → `talon init` detects this and sets `backend = "native"` with explicit warning; user can switch to Docker later
- Seccomp on macOS → Docker Desktop handles transparently; document
- Native mode misuse → mitigated by always-Dangerous approval level and persistent `[NATIVE]` labelling

---

## Phase 4 — Gateway (Weeks 5–6)

> **Edge:** Telegram + CLI + HTTP from one binary, unified session memory. Start in Telegram,
> continue in CLI — same context. Build HTTP gateway first (testable without bot tokens), Telegram
> second, Discord last (serenity has heavy deps; sequence to reduce integration risk).
>
> **TUI edge:** The first AI agent TUI in Rust with streaming markdown, syntax-highlighted diffs,
> adaptive layout, and inline images. Study reference: OpenCode (Go/Bubbletea) for UX parity.
> See `docs/10_TUI/` for full research (docs 77–79).

### TUI Architecture

**Pattern:** MVU (Model-View-Update, Elm-style) — same pattern as Bubbletea/OpenCode.
All async events (LLM tokens, tool results, keyboard) flow through `mpsc` channels into a single
update loop. No shared mutable state. Render is pure: `View(Model) → Frame`.

```
tokio runtime
  ├── LLM stream     ─┐
  ├── Tool executor  ─┼──mpsc──▶ MVU loop ──▶ Ratatui ──▶ Crossterm
  └── Crossterm keys ─┘          │
                                 └──▶ Model state (no Mutex needed)
```

**Five components** (see doc 78 for full spec):

| Component | What it renders |
|-----------|----------------|
| `ChatView` | Streaming markdown — `comrak` AST → `syntect` highlights → ratatui Spans |
| `InputBar` | Multi-line input (`tui-textarea`), history (↑↓), autocomplete |
| `ToolPanel` | Collapsible side/bottom; active tool spinners; expandable output with syntax highlight |
| `StatusBar` | Model name, token count, session id, sandbox mode (`[NATIVE]` badge if native) |
| `SplitPane` | Adaptive: `<80 cols` → stacked compact; `≥120 cols` → ChatView + ToolPanel side-by-side |

**Render modes** (detect at startup, `--gateway` flag overrides):

| Mode | When | How |
|------|------|-----|
| `TUI` | Interactive terminal | Full ratatui with all components |
| `Accessible` | `--accessible` or `--no-tui` | Line-by-line, no escape sequences, `indicatif` spinners |
| `Plain` | `NO_COLOR`, `$TERM=dumb`, piped stdin | Raw text, no colour, works in CI |

**Streaming markdown approach** (LLM tokens arrive one by one):
1. Buffer incoming tokens into a `String`
2. Parse with `comrak` on every render frame (~60fps)
3. Detect unclosed blocks (code fence, list) — show with dim `…` indicator
4. Re-parse when buffer grows; complete elements render normally

### Tasks

**Gateway foundation**
- [ ] 4.1 `crates/talon-gateway/src/lib.rs` — `Gateway` trait + normalized `Message` struct + `RenderMode` enum
- [ ] 4.2 `crates/talon-gateway/src/normalize.rs` — markdown normalization per platform (Telegram strips some syntax, TUI renders all)
- [ ] 4.3 `crates/talon-gateway/src/cli.rs` — `CliGateway`: stdin/stdout loop, `indicatif` spinner while agent thinks
- [ ] 4.4 `crates/talon-gateway/src/http.rs` — `HttpGateway` (axum): `POST /v1/messages`, SSE stream `GET /v1/stream/:session_id` — **build this first; no bot token required**

**TUI gateway**
- [ ] 4.5 `crates/talon-gateway/src/tui/mod.rs` — `TuiGateway`: startup capability detection (`detect_capabilities()` → `RenderMode`), raw mode init, event loop spawn
- [ ] 4.6 `crates/talon-gateway/src/tui/app.rs` — `App` struct (MVU model): `messages: Vec<Message>`, `input: TextArea`, `tool_calls: Vec<ActiveTool>`, `layout: LayoutMode`; `update(Msg) -> App` is pure
- [ ] 4.7 `crates/talon-gateway/src/tui/components/chat.rs` — `ChatView`: streaming markdown renderer; `comrak` AST → ratatui `Text`; `syntect` for fenced code blocks; OSC 8 links; inline images via `ratatui-image` (auto-detects Kitty/iTerm2/Sixel/halfblocks)
- [ ] 4.8 `crates/talon-gateway/src/tui/components/input.rs` — `InputBar`: `tui-textarea` for multi-line edit; Ctrl+Enter to submit; ↑↓ history; `/` command autocomplete
- [ ] 4.9 `crates/talon-gateway/src/tui/components/tools.rs` — `ToolPanel`: collapsible (Tab to toggle); spinner per active tool; `similar`-powered diff view for `EditFileTool` proposals (red/green unified diff); expand/collapse individual calls
- [ ] 4.10 `crates/talon-gateway/src/tui/components/status.rs` — `StatusBar`: model name, token usage, session id, `[NATIVE]` sandbox badge, multiplexer detection (tmux/zellij prefix hint)
- [ ] 4.11 `crates/talon-gateway/src/tui/layout.rs` — `SplitPane` adaptive layout: `<80 cols` stacked, `≥120 cols` side-by-side; listens to terminal resize events
- [ ] 4.12 `crates/talon-gateway/src/tui/render.rs` — `detect_capabilities()`: checks `NO_COLOR`, `$TERM`, `--accessible` flag, pipe detection; returns `RenderMode`

**Remaining gateways**
- [ ] 4.13 `crates/talon-gateway/src/telegram.rs` — `TelegramGateway` (teloxide): polling + webhook modes
- [ ] 4.14 `crates/talon-tools/src/send_message.rs` — `SendMessageTool` (NeedsApproval): agent pushes to any channel
- [ ] 4.15 `crates/talon-gateway/src/registry.rs` — `GatewayRegistry`: `HashMap<ChannelId, Arc<dyn Gateway>>`
- [ ] 4.16 Update `talon/src/main.rs`: `--gateway cli,tui,telegram,http` flag; `--accessible` flag; spawn each as `tokio::spawn`
- [ ] 4.17 Integration tests: CLI roundtrip, HTTP POST roundtrip with mock LLM, TUI render smoke test (headless)
- [ ] 4.18 Manual test: Telegram bot responds within 5s end-to-end
- [ ] 4.19 `talon init` onboarding wizard — `talon/src/init.rs`: detect available provider(s) by probing auth (env vars, CLI tools); query each provider's models endpoint (e.g. `GET https://api.githubcopilot.com/models`); present an interactive numbered list of `model_picker_enabled` models; write chosen model to `~/.talon/config.toml [llm] model`; all providers read this config first, then `TALON_LLM_MODEL` env override, then their `DEFAULT_MODEL` constant as last-resort fallback. Run automatically on first launch when no config exists.

### New workspace dependencies (add in this phase)

```toml
tui-textarea    = "0.6"      # multi-line input widget
ratatui-image   = "2"        # inline images (Kitty/Sixel/iTerm2/halfblocks)
comrak          = "0.28"     # CommonMark/GFM parser → AST
syntect         = "5"        # syntax highlighting (Sublime grammars)
similar         = "2"        # diff algorithm for file change display
indicatif       = "0.17"     # spinners/progress bars (non-TUI mode)
inquire         = "0.7"      # interactive prompts (non-TUI mode)
strip-ansi-escapes = "0.2"   # clean output for logging/accessibility
unicode-width   = "0.1"      # correct layout with CJK/emoji
```

### Exit Gate
```bash
cargo nextest run -p talon-gateway
# CLI mode
cargo run --release -- --gateway cli
# TUI mode — interactive
cargo run --release -- --gateway tui
# Accessible fallback
cargo run --release -- --gateway tui --accessible
# Piped (plain mode auto-detected)
echo "hello" | cargo run --release -- --gateway cli
# HTTP
curl -X POST http://localhost:7777/v1/messages -d '{"content":"hi"}'   # 200 OK
# Telegram
# Set TELEGRAM_BOT_TOKEN, send "hello" → response <5s
# Diff rendering: propose an EditFile, verify red/green diff in ToolPanel
```

### Risks
- Streaming markdown flicker — mitigate with frame-rate cap (60fps) and incremental `comrak` parsing
- `ratatui-image` protocol detection fails on unusual terminals — halfblocks fallback always works
- `tui-textarea` unicode handling edge cases (CJK, emoji) — `unicode-width` crate handles layout
- tmux/zellij wraps escape sequences — detect multiplexer and disable image rendering inside it

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

## Phase 2.5 — Talon LTM + LanceDB Memory Layer (Week 4–5, parallel with Phase 3)

> **Edge:** Two-tier memory (working + long-term) with automatic fact extraction and semantic
> deduplication. No other open-source agent does this in a single Rust binary.
>
> **Architecture decision (final):** LanceDB is the memory storage engine. SQLite remains for
> sessions/config/coordination. Redis is NOT a dependency. The concepts from Redis Iris
> (two-tier memory, fact extraction, semantic dedup, hybrid search, semantic cache) are implemented
> in pure Rust via Talon LTM + LanceDB. See `CLAUDE.md` Memory Stack section for the full decision.
>
> **Talon LTM** = own Rust crate (`crates/talon-memory/src/ltm/`), claude-ltm blueprint (doc #72).
>   Memory model: categories, importance 1–5, decay, FTS5-first search, auto-extraction.
> **LanceDB** = embedded vector + FTS + hybrid search. No server. See doc #73.
> **Honker** = optional reactive layer (queues, NOTIFY/LISTEN, cron) — add post-v1.0. See doc #76.

### Tasks
- [ ] 2.5.1 Add `lancedb`, `arrow-array`, `tokio-stream` to `[workspace.dependencies]`
- [ ] 2.5.2 `crates/talon-memory/src/lance_store.rs` — `LanceMemoryStore` impl of `MemoryStore` trait: LanceDB table `memories` with columns `(id, content, category, importance, created_at, accessed_at, decay_score, embedding BLOB)`. FTS via LanceDB's built-in full-text index.
- [ ] 2.5.3 `crates/talon-memory/src/ltm/mod.rs` — **Talon LTM** memory model: `Memory { id, content, category, importance: u8 (1–5), decay_score: f32, tags, entities }`. Categories: `user_preference`, `decision`, `fact`, `pattern`, `gotcha`.
- [ ] 2.5.4 `crates/talon-memory/src/working.rs` — `WorkingMemory` struct: token-budgeted message window, auto-summarizes via LLM call when budget exceeded (claude-ltm two-tier pattern, doc #67)
- [ ] 2.5.5 `crates/talon-memory/src/facts.rs` — `FactExtractor`: LLM-powered extraction per session end; produces `Vec<Memory>` with category, importance score, entities
- [ ] 2.5.6 `crates/talon-memory/src/dedup.rs` — Semantic deduplication: embed new memories, cosine-compare against existing (threshold 0.85), merge duplicates instead of appending
- [ ] 2.5.7 `crates/talon-memory/src/promotion.rs` — Memory promotion: post-session hook moves high-importance working memory facts → LanceDB long-term store
- [ ] 2.5.8 `crates/talon-memory/src/hybrid_search.rs` — Hybrid retrieval: LanceDB vector KNN + FTS, fused with Reciprocal Rank Fusion (RRF)
- [ ] 2.5.9 `crates/talon-memory/src/cache.rs` — `SemanticCache`: embed LLM prompts, return cached response if similarity > 0.95. LRU in-memory with TTL. No Redis. (Iris LangCache pattern, doc #70)
- [ ] 2.5.10 `crates/talon-memory/src/decay.rs` — `DecayEngine`: time-based importance decay, run as periodic task (once per day, not per query)
- [ ] 2.5.11 Update `ContextBuilder` (Phase 2 task 2.7) to use `WorkingMemory::compact()` for auto-summarization instead of static window trimming
- [ ] 2.5.12 Integration tests: fact extraction round-trip, dedup merges similar facts, hybrid search returns ranked results, semantic cache hit/miss, decay reduces importance over time
- [ ] 2.5.13 `talon cache clear` + `talon cache stats` + `talon memory stats` CLI subcommands

### Exit Gate
```bash
cargo nextest run -p talon-memory
cargo run --release -- --message "remember that I prefer dark mode"
# then in new session:
cargo run --release -- --message "what do you know about my preferences?"
# expects: recalls "prefers dark mode" via LanceDB hybrid search
```

### Risks
- LLM cost for automatic fact extraction — mitigate with semantic cache + batch extraction (once per session end, not per turn)
- LanceDB Rust API stability (v0.9, pre-1.0) — pin version, test on upgrade
- Embedding model size — `fastembed` adds 30–60 MB; keep behind `semantic-search` feature flag
- Fact extraction quality — LLM determines what's worth remembering; tune the extraction prompt carefully

---

## Graphify Health — Action Items (from graph analysis, 2026-05-28)

> Generated from graphify graph analysis (3,806 nodes, 3,937 edges, 264 communities).

### 🔴 ~~Critical: Test Coverage Gaps~~ — RESOLVED (false alarm)

> Initial grep only matched `#[test]`, missing `#[tokio::test]`. Actual counts: agent.rs=8, context.rs=6, dispatcher.rs=10, sqlite_store.rs=5, store.rs=3. Total: 190 tests across 22 files.

- [x] GH.1 ~~Write tests for `agent.rs`~~ — already has 8 tests (tokio::test)
- [x] GH.2 ~~Expand tests for `context.rs`~~ — already has 6 tests
- [x] GH.3 ~~Expand tests for `dispatcher.rs`~~ — already has 10 tests
- [x] GH.4 ~~Add tests for `sqlite_store.rs` and `store.rs`~~ — already have 5 and 3 tests

### 🟡 Architecture Cleanup

- [x] GH.5 Unify 3 conflicting `ApprovalLevel` enum definitions across spec docs — only `crates/talon-core/src/approval.rs` is canonical
- [x] GH.6 Fix remaining `Arc<Box<dyn Tool>>` references in docs — canonical form is `Arc<dyn Tool>` (Type #5) — verified: all occurrences are already corrective/reference context, no live misuse
- [x] GH.7 Archive dead research docs (Redis/Iris, competitive analysis) to `docs/archive/` — moved: 09_Redis_Iris (8 docs), 4 orphan feature audits, 2 orphan migration docs, dogfood-output (9 audit reports)

### 🟢 Maintenance

- [x] GH.8 Re-run `graphify update .` after archiving docs — nodes 3,806→2,879 (−24%), edges 3,937→3,045 (−23%), communities 264→213 (−19%)
- [x] GH.9 Cross-reference god nodes with test coverage — `open_db()` / `Database::open` is well-tested (15+ test functions across lib.rs, context.rs, sqlite_store.rs, integration.rs, agent.rs)

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
- [ ] Two-tier memory (talon-ltm + LanceDB): auto fact extraction + semantic dedup operational
- [ ] Semantic cache reduces repeated LLM calls (verified with cron test)
- [ ] LanceDB hybrid search (vector KNN + FTS + RRF) returns ranked results across sessions

## Beat the Competition

- [ ] **vs Claude Code:** Persistent cross-project FTS5 memory, Telegram + Discord, single pre-built binary via `curl | sh`
- [ ] **vs Hermes:** zero GIL, single binary, unified cross-channel memory, no venv needed
- [ ] **vs OpenClaw:** Rust binary vs NestJS, WASM plugins vs npm, queryable session store vs stateless
- [ ] **vs Aider:** persistent FTS5+semantic memory, multi-channel, skill evolution (v2)
- [ ] **vs Goose:** richer tools (browser, MCP, evolution), WASM hot-reload, cross-channel sessions
