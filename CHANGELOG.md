# Changelog

All notable changes to the Talon project will be documented in this file.

## [Unreleased]

### Added — Phase 8: Flow Cottage (secrets, auth, webhooks, reliability, observability)
- **`talon-secrets` crate (8th workspace member)** — `SecretProvider` trait, `{{secret:NAME}}` / `secret://provider/path#key` references, and a builtin vault: AES-256-GCM envelope encryption in `talon.db` (migration v6), per-secret data keys wrapped by a master key that exists **only behind an unlock credential** — OS keychain, argon2id passphrase recovery blob, or `TALON_MASTER_KEY` (headless). `talon init` gains the credential step (credential *before* keygen; abort leaves zero key material); `talon secret set/get --reveal/list/rm/rewrap` CLI; `rewrap` rotates wraps without re-encrypting stored secrets.
- **Just-in-time resolution + redaction choke points** — job prompts resolve secrets at dispatch only; an unresolvable reference fails the run *before* the LLM is called, naming the reference, never a value. Resolved values are registered for the run's lifetime and scrubbed (`[REDACTED:NAME]`) from run records, all tracing output, the log file, the console log tail, and every SSE frame.
- **External secret providers (read-only, feature-gated)** — HashiCorp Vault KV v2 with AppRole auth (`vault` feature; token caching + 403 re-login) and AWS Secrets Manager via the SDK default credential chain (`aws-secrets` feature; `#json-key` extraction). Default builds compile neither — enforced by `scripts/check_default_features.sh` in CI.
- **Named API tokens with roles (migration v7)** — `talon token create NAME --role admin|viewer` (+ `/api/v1/tokens`): SHA-256-hashed at rest, shown exactly once, revocable; viewers are read-only (mutations → 403). `GET /api/v1/me` returns the caller's identity; the legacy `[gateway] api_token` keeps working as an implicit admin with a startup deprecation warning. Console gains a real login view that validates the token against `/me` **before** persisting it, and hides every mutating affordance from viewers.
- **Webhook triggers (migration v8)** — `POST /api/v1/jobs/{id}/hooks` registers a hook whose signing secret lives in the builtin vault (returned once). The public `POST /hooks/{id}` endpoint — the binary's only unauthenticated route — verifies HMAC-SHA256 over `timestamp.body` (constant-time, 5-minute replay window), enforces a per-hook rate limit (`[webhooks] rate_per_min`) and a 64KB body cap, then queues an immediate run with the JSON payload exposed to the agent as context. Run provenance is recorded in `cron_runs.fired_by` (`cron`/`manual`/`webhook`/`failure`).
- **Run reliability (migration v9)** — per-job `retry_max` (exponential backoff with jitter, each attempt its own `cron_runs` row) and `on_failure` error-handler jobs that receive a redacted failure-context block; handlers can never cascade (`fired_by = "failure"` guard) and self-reference is rejected at the store, covering API, CLI, and flow commits alike. The console run timeline shows `fired_by` + attempt badges.
- **Observability** — `[logging]` config: scrubbed pretty/JSON stderr plus a JSON file sink with daily rotation under `~/.talon/logs/`; every run executes inside a `run` span (`job_id`, `run_id`) and every HTTP request gets an `x-request-id`; token-protected Prometheus `GET /metrics` (`talon_runs_total{status}`, run-duration histogram, active-jobs gauge); live `GET /api/v1/logs/tail` SSE with level filter + console Logs page; opt-in OTLP trace export behind the `otel` feature (`[otel] endpoint`).
- **Graph editor v2** — the execution graph is now editable: drag node→node to add a `context_from` dependency, Delete removes it; the server re-validates the whole graph (unknown ids, self-reference, cycles → 422). Auto-layout button, persisted node positions, minimap, and a live amber pulse on running nodes.
- **Hardening (migration v10)** — `[runs] retention_days` daily prune of completed run history (crashed `running` rows are kept as forensic signal) and an `audit_log` recording every mutating API call with an 8-hex token fingerprint (attributable to named tokens, never a usable credential).
- **Live e2e suite** — `scripts/e2e_smoke.sh` boots a real `talon serve` in an isolated `$HOME` and asserts the entire surface (23 checks), including two leak canaries: a stored secret value must never appear in run records or the server log.

### Fixed — Phase 8
- `aws-config`/`aws-sdk-secretsmanager` now use the `default-https-client` (rustls 0.23) instead of the legacy `rustls` feature, removing rustls-webpki 0.101 and its three RUSTSEC advisories (2026-0098/0099/0104) from the lockfile.
- A machine with only `TALON_MASTER_KEY` (no keychain entry, no recovery blob) now gets a working vault in both the daemon and the `talon secret` CLI — the headless path was previously gated behind local bootstrap state.

### Added — Phase 7: Web Console ("Jenkins + n8n for AI agents")
- **`cron_runs` per-run history (migration v5)** — one row per execution attempt (`running/success/failure/timeout/skipped/denied`, output, error, `AgentEvent` transcript). `RunStore` in `talon-memory`; the scheduler records the lifecycle around the `JobRunner` seam. `cron_jobs.last_run` crash semantics unchanged; without `with_run_store` the scheduler behaves exactly as before.
- **Manual trigger** — `SchedulerCmd::Trigger` over a `SchedulerHandle`: runs a job immediately without advancing `next_run`/`run_count` (Jenkins "Build Now").
- **`/api/v1` web console API** (`talon-gateway::web`) — jobs CRUD + trigger, run history, `GET /graph` (nodes + `context_from` edges), SSE `GET /events` (`RunEvent` broadcast), approvals inbox (`ApprovalBroker` resolves §4.4 out-of-scope escalations from the browser; 120s timeout → deny). Bearer-token auth on every route, fail-closed (no token in config → API not mounted); SSE accepts `?token=` (EventSource limitation, localhost-default bind).
- **AI flow builder endpoints** — `POST /flows/plan` (NL → LLM-drafted DAG of cron jobs; croner-validated, scope predicted per job via `predict_scope`; writes nothing) and `POST /flows` (commits an approved draft: strips `Dangerous`-class tools server-side, topo-sorts, remaps draft keys to real job ids).
- **Embedded SPA** (`web/`, React + Vite + React Flow; committed `web/dist` served at `/ui` via `rust-embed` behind the opt-in `web-ui` feature) — Dashboard (web twin of `talon cron list`), execution Graph (node color = last run status), Job detail (run timeline + transcripts + scope), Flow Builder (draft + grant-box checklist), Approvals inbox. Rebuild assets with `bun run build` in `web/`.
- **`talon init`** now auto-generates `[gateway] api_token` when absent (existing tokens preserved).

### Added — Phase 2.5: Talon LTM (SQLite + sqlite-vec)
- **`talon-ltm` memory layer** — long-term memory implemented natively in Rust over a single SQLite database (ADR 0008 — supersedes the earlier LanceDB plan; `sqlite-vec` for vectors + FTS5 for keyword + RRF fusion in Rust, no LanceDB/Redis):
  - `sqlite-vec` extension wired into the `deadpool-sqlite` pool (2.5.1)
  - `LtmStore` over SQLite — `memories` table + `memories_fts` (FTS5, porter stemming) + `vec_memories` (2.5.2)
  - Typed `MemoryCategory` enum + tags on the LTM model (2.5.3)
  - Token-budgeted `WorkingMemory` with rolling summary (2.5.4)
  - LLM-powered `FactExtractor` with a Markdown extraction prompt (2.5.5)
  - Semantic deduplication of memories (2.5.6)
  - `Promoter` — promotes high-importance session facts to LTM (2.5.7)
  - `HybridSearch` — hybrid FTS5 + vector retrieval via Reciprocal Rank Fusion, k=60 (2.5.8)
  - `SemanticCache` — semantic LLM response cache (2.5.9)
  - `DecayEngine` — time-based memory decay (2.5.10)
  - `ContextBuilder` folds overflow turns through `WorkingMemory` (2.5.11)
  - End-to-end LTM integration tests (2.5.12)
- **CLI:** `talon memory` (stats) and `talon cache` (stats/clear) subcommands (2.5.13)
- **Agent runtime wiring (2.5.14)** — LTM is now live in the agent loop: FTS5 recall at the start of each turn (injected into the system prompt) and automatic LLM fact-extraction → promotion at turn end. Recall queries are sanitized via `fts5_or_query` to keep raw user text from breaking `MATCH`. Live cross-session recall verified end-to-end through the key-less `github-copilot` provider.

### Added
- **Redis Iris integration docs** (`docs/09_Redis_Iris/`) — 5 new documents exploring Redis Iris as Talon's context engine:
  - `66_Redis_Iris_Overview.md` — Strategic fit analysis, three integration options (SQLite-only / Redis-only / Hybrid), recommendation for Option C (feature-flagged)
  - `67_Iris_Two_Tier_Memory.md` — Two-tier memory architecture (working + long-term), auto-summarization, LLM fact extraction, semantic deduplication, memory promotion
  - `68_Iris_Technical_Integration.md` — Rust integration strategies (sidecar vs native vs MCP), `redis` crate usage, Redis data model, feature flag design, performance benchmarks
  - `69_Iris_Philosophy.md` — Design principles ("context is all you need"), philosophical tension with single-binary story, what to adopt vs skip
  - `70_Iris_Semantic_Cache.md` — LangCache-inspired semantic response caching, cost optimization estimates, cache invalidation strategy
- **Phase 2.5 in PLAN.md** — Redis Iris Memory Layer: 12 tasks covering two-tier memory, fact extraction, semantic dedup, hybrid search, semantic cache, and optional Redis backend
- **roadmap.md** — Chronological implementation roadmap with dependency graph, week-by-week timeline, critical path analysis (~12 weeks to v1.0), priority stack for time-constrained builds, and risk register
- Updated `00_Master_Index.md` — Added section 09_Redis_Iris (5 docs), total doc count 65→70, completed count 9→14
- Updated Final Acceptance Criteria in PLAN.md — Added Iris memory, semantic cache, and Redis backend gates
- **Brain candidates research** (`docs/09_Redis_Iris/`) — 5 new documents evaluating memory/context engine candidates for Talon:
  - `71_Brain_Candidates_Overview.md` — Comparison matrix of 6 candidates (vector search, FTS, decay, auto-extraction, maturity)
  - `72_Claude_LTM_Analysis.md` — claude-ltm-plugin deep dive ★★★★★: categories, importance scoring, decay, typed memory graphs, auto-extraction, FTS5-first search — recommended as Talon's design blueprint
  - `73_LanceDB_Analysis.md` — LanceDB embedded vector+FTS DB ★★★★☆: SQLite-like but with native vectors, potential storage engine replacement
  - `74_Mem0_Rust_Analysis.md` — mem0-rust agent memory layer ★★★☆☆: multi-backend, auto-extraction, thin memory model
  - `75_Qdrant_Rig_Swiftide_Analysis.md` — Ecosystem players ★★–★★★: Qdrant (mature but needs server), Rig (clean traits), Swiftide (pipeline patterns)
- Updated `00_Master_Index.md` — Added docs 71–75, total doc count 70→75, completed count 14→19
- Updated graphify knowledge graph — 3026→3130 nodes, 2892→2989 edges, 206→214 communities
- **Emerging recommendation**: claude-ltm memory model as design blueprint + LanceDB as storage engine (feature-flagged: `sqlite-memory` default vs `lance-memory`)
- **Honker reactive layer** (`docs/09_Redis_Iris/76_Honker_Reactive_Layer.md`) — SQLite NOTIFY/LISTEN + durable queues + streams + scheduler as Talon's nervous system. Pairs with talon-ltm (own Rust reimplementation of claude-ltm blueprint) + LanceDB. Graph layer optional/later.
- Updated `00_Master_Index.md` — Added doc 76, completed count 14→15
- **Architecture decision:** claude-ltm is a **blueprint to reimplement** as `talon-ltm` in Rust, NOT a direct dependency. Honker adds reactive plumbing on top. Graph is optional.
- **TUI research & technology selection** (`docs/10_TUI/`) — 3 new documents:
  - `77_TUI_Landscape_Overview.md` — Comprehensive comparison of TUI frameworks (Ratatui, Ink, Textual, Bubbletea, Cursive) + how AI CLIs (Claude Code, OpenCode, Aider, Amazon Q) build their interfaces
  - `78_TUI_Technology_Selection.md` — Decision: Ratatui + Crossterm + MVU architecture. Component design (ChatView, InputBar, ToolPanel, StatusBar), async integration, adaptive layout, essential crates list
  - `79_Terminal_Rendering_Capabilities.md` — Image protocols (Kitty/Sixel/iTerm2), streaming markdown rendering, OSC 8 clickable links, accessibility (`NO_COLOR`, `--accessible`), multiplexer awareness, web hybrid (xterm.js), diff rendering
- Updated `00_Master_Index.md` — Added section 10_TUI (3 docs), completed count 15→18
- **Architecture decision: LanceDB from day one** — dropped `sqlite-memory` vs `lance-memory` feature flag. LanceDB is the sole memory backend (FTS + vectors + hybrid search). SQLite remains for non-memory concerns (sessions, config, Honker coordination). One backend, one path, no throwaway code.
- Updated docs 72, 73, 76 to reflect unified architecture: talon-ltm (claude-ltm blueprint) + LanceDB (storage) + Honker (reactive layer). Graph optional/later.
