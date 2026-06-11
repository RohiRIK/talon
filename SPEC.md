# Talon — System Spec: The Proactive Memory Agent

> Status: Draft v1 · 2026-06-01 · Peer to `PLAN.md` / `roadmap.md`
> Supersedes nothing — this is the **product framing** layer above the phase plan.
> **One-line thesis:** Talon is a single Rust binary that *remembers across time and acts on its own* — a scheduled, memory-driven agent that reaches you on your channels, with no cloud and no Python.

---

## 0. Why we're doing this

After a month of living with **various agent gateways that ship a cron scheduler**, the conclusion is concrete: **the scheduler is the killer feature.** Not the chat. Not the tool calls. The fact that the agent *wakes itself up*, remembers what happened last time, does the work, and pings you — without you asking — is what turns a tool into an assistant.

Everyone ships a request-response CLI agent. Almost nobody ships a **proactive** one in a single no-cloud binary. That gap is the product.

The two things that make a proactive agent worth trusting are the two things Talon *already built last week*:

1. **Cross-session memory** (`talon-memory` LTM — done) — so a 7am job knows what the 7am job did yesterday.
2. **Multi-channel reach** (Telegram / Discord / HTTP gateways — done) — so the agent can find you.

The scheduler is the missing third leg. Adding it is **repositioning, not a rewrite**: no ADR is invalidated, no load-bearing type changes. We change the *emphasis* — from "a CLI agent that has memory" to "an always-on memory agent that also has a CLI."

---

## 1. The repositioning (what changes, what doesn't)

| | Before (CLI-first) | After (daemon-first) |
|---|---|---|
| Primary surface | `talon --message "..."` (one-shot) | `talon serve` (always-on daemon) |
| Identity | Chat agent with memory | Proactive agent that schedules itself |
| CLI role | The product | One client among several |
| Memory | Queried on demand | Acted on autonomously |
| What's locked | 7 load-bearing types, ADR 0008, single-binary thesis — **all unchanged** | same |

**Nothing in the locked architecture moves.** `talon serve` is additive: it boots the same provider, DB, tools, and gateways that already exist, and adds a scheduler task alongside the gateway loop. One-shot `--message` keeps working untouched.

---

## 2. The three layers

```
┌─────────────────────────────────────────────────────────────┐
│  CONSUMERS — Claude Code & other CLI agents   [via Layer 2]   │
│  Schedule Talon jobs / query Talon memory from their sessions │
├─────────────────────────────────────────────────────────────┤
│  LAYER 2 — MCP Server   [after Layer 1]                       │
│  Expose cron + memory as MCP tools other CLI agents call      │
├─────────────────────────────────────────────────────────────┤
│  LAYER 1 — Talon Core Daemon   [BUILD NOW]                    │
│  talon serve = Scheduler + LTM + Tools + Gateways, always-on  │
└─────────────────────────────────────────────────────────────┘
```

Decision (locked this session): **Daemon-first, MCP later.** The scheduler core is identical under all packaging choices, so we build it first and wrap it afterward. There is no separate "cloud" layer — "other CLI agents" reach Talon **through the MCP server** (Layer 2).

---

## 3. What we already have (inventory)

Phases 0–5 **and** 2.5 complete. `cargo nextest run --workspace` → 383/383 green.

| Crate | Provides | Relevant to scheduler? |
|---|---|---|
| `talon-core` | Agent loop (`Agent::run(session_id, msg)`), `ApprovalMembrane`, `AgentEvent`, `AgentState`, `ToolDispatcher` (sequential default) | **Yes** — scheduler invokes `Agent::run` |
| `talon-memory` | LTM (`sqlite-vec` + FTS5 + RRF), `WorkingMemory`, `FactExtractor`, dedup, `Promoter`, `HybridSearch`, `SemanticCache`, `DecayEngine`, `ContextBuilder`, `Database` pool | **Yes** — new `CronStore` lives here; jobs recall LTM |
| `talon-llm` | `LlmProvider` trait + impls (Codex, ClaudeCode, Antigravity, keyless github-copilot, …) | Yes — jobs run through a provider |
| `talon-tools` | fs, terminal (docker/native), web search/extract, browser (feat), MCP client/adapter, subprocess plugin, send_message, session_search | Yes — new `CronJobTool` lives here |
| `talon-gateway` | `cli`, `http`, `telegram`, `tui`, `registry`, `web` (Phase 7: `/api/v1` console API + embedded `/ui` SPA behind `web-ui` feature); `GatewayContext::build_agent(event_tx)` constructs a wired `Agent` | **Yes** — daemon reuses this; output routes to a channel |
| `talon-plugins` | WASM host (wasmtime) — Phase 6, **not started, not needed for the scheduler** | No |

**Key construction path to reuse:** `GatewayContext::build_agent(event_tx)` (crates/talon-gateway/src/lib.rs:132) already assembles provider + dispatcher + db into an `Agent`. The scheduler builds jobs through this exact path — no new wiring of the agent internals.

---

## 3.5 Prior art & engine decision (research · 2026-06-01)

### What the source agents do

| Source | Cron impl | Persistence | Deliver abstraction worth keeping |
|---|---|---|---|
| Hermes (Python) | `cron/scheduler.py` + `cron/jobs.py` | SQLite, survives restart | `origin` / `local` / `all` / `platform:chat_id:thread_id` |
| OpenClaw (TS) | `node-cron` + SQLite state | SQLite | per-profile isolation |

Both are SQLite-backed, persist across restart, and isolate cron per profile. Hermes' **deliver-target string** (`platform:chat_id:thread_id`) is the cleanest single idea — adopt it verbatim instead of my earlier flat `channel`. Talon's own `docs/04_Core_Features/33_Cron_Scheduler.md` already designed a Rust port with a rich, human-readable `CronSchedule` enum (`Cron("0 9 * * *")` / `Human("every 2h")` / `Once(ts)`), a job-output **DAG** (`context_from` — one job feeds another), and repeat/one-shot counts. **Keep that data model.**

### Rust scheduling crates compared (June 2026)

| Crate | Role | Persistence | Verdict for Talon |
|---|---|---|---|
| **tokio-cron-scheduler** 0.15 (Oct 2025, 542k dl/mo) | full scheduler | Postgres / Nats — **no SQLite** | ❌ persistence mismatch → forces a 2nd source of truth |
| **SACS** 0.9 (Apr 2026, 175 KB) | lightweight scheduler | in-memory only | ❌ no persistence |
| **apalis** | job-queue framework | sqlite/pg/redis, non-standard cron | ❌ overkill, wrong abstraction |
| **croner** / cron-lite | cron *parser* (compute `next_run`) | n/a — we store | ✅ pair with our SQLite store |

### Decision — revises doc 33's *engine*, keeps its *data model*

**The SQLite `CronStore` is the single source of truth.** Use **croner** only to parse expressions and compute `next_run`. A thin tokio tick-loop queries `due(now)` and dispatches. We **do not** add `tokio-cron-scheduler`: its persistence is Postgres/Nats, which cannot live in `talon.db`, so it would force a second store and break the one-file thesis. Doc 33 paired it with a *separate* SQLite store anyway — the library was only ever the timing engine, and a ~50-line tick-loop we control replaces it.

Why this is the most human-readable architecture the user asked for: **one** state store, **one** data flow (`tick → due → run → mark → recompute`), an engine small enough to read in a sitting, and natural-language schedules (`"every 2h"`) at the surface.

### The Tokio question, resolved

"Is Tokio the right choice?" is really two questions:

- **Tokio the async runtime** — settled by `docs/06_Concurrency/49_Tokio_Runtime_Design.md` (I/O-bound workload, ecosystem standard; `async-std`/`smol`/threads all rejected) and **load-bearing across all 6 crates**. Replacing it means rewriting the project — not on the table.
- **`tokio-cron-scheduler` the *library*** — the thing actually worth dropping, and we *are* dropping it (above). We build the scheduler ourselves **on** the tokio runtime we already have.

So: **keep the runtime, skip the library.** That, I believe, is the concern behind "Tokio isn't good for me" — and it's the right instinct, aimed one layer too low.

---

## 4. LAYER 1 — Talon Core Daemon (build now)

### 4.1 Components

| ID | Component | File | Approval |
|---|---|---|---|
| S1 | `CronStore` — persisted jobs table + due-query | `crates/talon-memory/src/cron.rs` | — |
| S2 | `Scheduler` — tokio ticker, polls due jobs, runs them | `crates/talon-core/src/scheduler.rs` | — |
| S3 | `CronJobTool` — create (via the §4.4 scope wizard) / list / delete jobs via NL | `crates/talon-tools/src/cronjob.rs` | NeedsApproval |
| S4 | `talon serve` — daemon entrypoint, spawns gateways + scheduler | `talon/src/main.rs` | — |
| S5 | Delivery routing — job output → `deliver_to` target (`origin`/`local`/`all`/`platform:chat_id:thread_id`) | reuse gateway send paths | — |
| S6 | `talon cron list` — CLI that **renders jobs as a tree** (ships in the binary) | `talon/src/cron_cli.rs` | — |

**S6 — the tree view.** A first-class CLI subcommand (peer to the existing `talon memory` / `talon cache`). It renders jobs as a tree, using the job **DAG** as the natural hierarchy: root jobs at the top, `context_from` children nested beneath the job that feeds them. Each node shows its status glyph, name, humanized next run, schedule, `deliver_to`, and last-run status. Actual output (`talon cron list`):

```
● morning-brief  in 8h  (0 8 * * *)  → telegram:me  [ran 1d ago]
  ● follow-up-email  in 22m  (every 30m)  → telegram:me  [ran 3h ago]
● hourly-inbox-scan  in 9m  (0 * * * *)  → local  [ran 3h ago]
○ weekly-report  —  (0 9 * * 1)  → all  [never run]
```

Per-node format: `{glyph} {name}  {next-run}  ({schedule})  → {deliver_to}  [{last-run}]`. `●` enabled · `○` disabled · two-space indent = `context_from` dependency. Next-run humanizes to `in 2h` / `overdue` / `—` (never run); last-run to `ran 3h ago` / `never run`. Hand-rolled — no tree-drawing dependency. The renderer is a pure function over `(jobs, now)`, so it is unit-tested without wall-clock or a DB, and a dependency cycle can never hide a job (orphans are surfaced at the root).

These are tasks **6.6, 6.7, 6.8, 6.11** in `PLAN.md`, **pulled out of Phase 6** (which otherwise bundles unrelated WASM work) into a standalone mini-phase. WASM (6.1–6.5, 6.9, 6.10) stays in Phase 6 and is **not** a dependency.

### 4.2 Data model (adopted from doc 33 + Hermes deliver targets)

`CronStore` table (single `talon.db`, same file as everything else):

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | uuid v4 |
| `name` | TEXT | human label, nullable |
| `schedule` | TEXT (JSON) | `CronSchedule` enum: `Cron("0 9 * * *")` / `Human("every 2h")` / `Once(ts)` |
| `prompt` | TEXT | the instruction run as the agent's user message |
| `session_id` | TEXT | stable per job → LTM continuity across runs |
| `deliver_to` | TEXT | Hermes-style target: `origin` / `local` / `all` / `platform:chat_id:thread_id` |
| `context_from` | TEXT (JSON) | upstream job IDs whose last output is injected (job DAG) |
| `granted_scope` | TEXT (JSON) | capability allowlist set by the creation wizard (§4.4): tool names + Bash command patterns |
| `enabled` | INTEGER | 1/0 |
| `tz` | TEXT | IANA tz (default from config) |
| `repeat` | INTEGER | `NULL` = infinite, `1` = one-shot, `n` = n runs |
| `run_count` | INTEGER | increments per fire |
| `last_run` | TEXT | ISO8601, nullable |
| `last_output` | TEXT | for `context_from` injection downstream |
| `next_run` | TEXT | ISO8601, computed by `croner` from `schedule` + `tz` |
| `created_at` | TEXT | ISO8601 |

`CronSchedule::Human` parses friendly forms (`"30m"`, `"every 2h"`, `"daily"`) → a cron string. This is the "highly readable for humans" surface — users never need to write raw cron unless they want to.

Methods: `create`, `list`, `delete`, `set_enabled`, `due(now) -> Vec<Job>`, `mark_run(id, ran_at, output, next_run)`. All via `spawn_blocking` / `deadpool-sqlite` — never hold a `Connection` across `.await` (load-bearing rule).

### 4.3 Scheduler behavior

- **Engine: self-rolled tick-loop, not `tokio-cron-scheduler`** (see §3.5). A tokio interval ticker (default 30s; configurable). On each tick: `CronStore::due(now)` → for each job, build an `Agent` via `GatewayContext::build_agent`, call `Agent::run(job.session_id, job.prompt)`, route emitted `AgentEvent`s to `job.deliver_to`, then `mark_run` and recompute `next_run`.
- **Cron parsing:** add one small dependency — **`croner`** — used only to validate expressions and compute `next_run`. No external scheduler state; SQLite is the only source of truth.
- **Concurrency:** jobs run sequentially per tick by default (safe); a bounded `JoinSet` (cap 4) is an opt-in later. A single slow job must not block the ticker — run job execution in a spawned task, ticker just dispatches.
- **Process model:** the scheduler **only runs under `talon serve`.** One-shot `--message` never starts it.

### 4.4 Approval under automation — RESOLVED (A + B via a creation-time wizard)

This is the single most important design point in this spec. `ApprovalMembrane` today assumes a **human is present**. A cron job runs **unattended** at 7am. The resolution: **safe-by-default + a pre-authorized scope, where the scope is computed by a wizard when the job is created.**

**Creation wizard flow** (runs once, with you present):
1. You describe the job in natural language — *"every morning, summarize my unread emails and post to Telegram."*
2. The wizard does a **dry-analysis pass**: the LLM inspects the prompt and predicts the exact capability set the job will need — which tools (`read_file`, `terminal`/Bash, `web_search`, `send_message`…), and for **Bash specifically, the concrete command patterns** it expects to run.
3. It presents that predicted scope as an **editable checklist** — the "grant box." You confirm or trim it. This confirmation is itself the `NeedsApproval` gate, satisfied *here*, with a human watching.
4. The approved set is persisted on the job as `granted_scope`.

**Runtime enforcement** (unattended):
- A tool call **inside** `granted_scope` → runs without prompting (**B** — efficient; you already approved it).
- A tool call **outside** scope (drift, prompt injection, an unforeseen tool) → never runs silently. It suspends and fires an **async ✅/❌ to `deliver_to`** (**A** — the Phase-4 Telegram keyboard already exists). Times out → *"skipped: out of granted scope."*
- `Dangerous`-class tools are **never** auto-granted by the wizard — they escalate async every time, even if predicted.
- Bash scope stores **command patterns (allowlist)**, not a blanket "may run Bash" — a job granted `git pull` cannot later `rm -rf`.

**Why it's both efficient and safe:** the common case (the job does exactly what you set it up to do) runs friction-free; the dangerous case can never execute while you're asleep. And the scope is **legible at creation** — you see *"this job can run Bash: `git pull`; read `~/notes/**`"* before it ever fires.

### 4.5 Resilience requirements

| Concern | Requirement |
|---|---|
| Restart | Jobs persist in SQLite; on boot, recompute `next_run`, resume ticking |
| Downtime / sleep | **Missed-run policy** (decision in §8): catch up once, or skip to next. Default proposal: skip, log a `missed_run` note the agent can mention |
| Crash mid-job | `last_run` only set on success; a crashed job re-fires next tick (idempotency is the job author's concern, surfaced in docs) |
| Shutdown | `talon serve` handles SIGTERM: stop accepting ticks, let in-flight jobs finish (bounded grace), flush, exit clean |
| Clock | Cron evaluated in the job's `tz`; DST handled by the cron crate |

### 4.6 Acceptance criteria (what must work)

- [ ] `*/1 * * * *` job fires within the target minute (PLAN 6.11)
- [ ] A scheduled job **recalls LTM from a prior run** and references it (cross-run memory — the whole point)
- [ ] Job output is **delivered to Telegram** end-to-end
- [ ] `CronJobTool` create / list / delete works; NeedsApproval gating enforced on each
- [ ] Jobs **survive a daemon restart** (persisted, `next_run` recomputed)
- [ ] The **creation wizard** predicts a job's tool/Bash scope and persists `granted_scope`; an **in-scope** Bash command runs unattended, an **out-of-scope** one escalates async instead of running
- [ ] A `Dangerous`-class tool inside a job **always** triggers async approval on `deliver_to` — never a silent run, never a hang
- [ ] `talon cron` renders the job list as a **tree** with `context_from` nesting and humanized next-run ("in 2h")
- [ ] `talon serve` shuts down cleanly on SIGTERM without dropping an in-flight job mid-write
- [ ] `cargo nextest run --workspace` green · zero `unwrap_used`/`expect_used` lints

---

## 5. LAYER 2 — MCP Server (confirmed)

Once the daemon core works, wrap it as an **MCP server** (`talon mcp` subcommand or a separate binary target) advertising tools:

| MCP tool | Backed by |
|---|---|
| `memory.recall(query)` | `talon-memory` `HybridSearch` |
| `memory.store(fact, category, importance)` | `Promoter` / `LtmStore` |
| `cron.schedule(expr, prompt, channel)` | `CronStore::create` |
| `cron.list` / `cron.delete(id)` | `CronStore` |

This makes Talon **infrastructure other agents plug into** — your Claude Code sessions could schedule a Talon job or query Talon's cross-project memory. Talon already *has* an MCP **client** (`talon-tools/src/mcp`); this adds the **server** side. Reuses 100% of Layer 1; no new core logic. **Not started until Layer 1 acceptance passes.**

---

## 6. Consumers — Claude Code & other CLI agents (confirmed)

This is what "other CLI agents" (D6) means: there is **no separate cloud layer.** Claude Code, Cursor, and any MCP-capable CLI agent reach Talon **through the Layer 2 MCP server** — the same single binary, running locally.

| Consumer | Flow | What they gain |
|---|---|---|
| Claude Code | adds `talon mcp` as an MCP server in its config | `cron.schedule(...)` a recurring job; `memory.recall(...)` Talon's cross-project LTM mid-session |
| Other CLI agents (Cursor, etc.) | same MCP handshake | proactive scheduling + persistent memory they don't have natively |
| Talon's own CLI/TUI | direct, in-process | full control + the `talon cron` tree view (S6) |

The win: Talon becomes the **memory + scheduler backend** for the CLI agents you already use, without any of them needing to reimplement either. Nothing here is "remote" or "cloud" — it's local MCP, consistent with the single-binary, no-cloud thesis.

---

## 7. Sequencing

```
NOW ──► Layer 1: Scheduler core (S1→S2→S3→S4→S5)         [days, not weeks]
        │  └─ exit: §4.6 acceptance criteria all green
        ▼
        Pending v1.0 verifications (parallel, cheap):
        Phase 5 live smoke (web_search + MCP) · 4.24 Telegram smoke
        ▼
        Layer 2: MCP server surface                       [after L1 green]
        ▼
        Phase 6 WASM plugins  ·  Layer 3 cloud (needs input)
```

Build order within Layer 1: `CronStore` (S1, testable in isolation) → `Scheduler` (S2, the engine) → `talon serve` (S4, wire it live) → `CronJobTool` (S3, NL surface) → delivery polish (S5). Each step is TDD'd with `cargo nextest`, committed on green, and ticks its box in `PLAN.md`.

---

## 8. Open decisions (need a call before / during build)

| # | Decision | Status / proposal |
|---|---|---|
| D1 | Scheduler engine | ✅ **Resolved (§3.5):** SQLite source-of-truth + `croner` parser + self-rolled tick-loop. No `tokio-cron-scheduler`. |
| D2 | Missed-run policy on downtime | Skip to next, log `missed_run` note |
| D3 | Approval-under-automation (§4.4) | ✅ **Resolved:** A+B — creation-time scope wizard predicts capabilities; in-scope runs free, out-of-scope escalates async; Bash stored as command allowlist |
| D4 | Tick interval | 30s default, configurable in `config.toml [scheduler]` |
| D5 | "MCP2" meaning (§5) | ✅ **Resolved:** Talon **as** an MCP server |
| D6 | "other CLI agents" meaning (§6) | ✅ **Resolved:** Claude Code & other CLI agents consume Talon via the MCP server — no cloud layer |
| D7 | `talon serve` vs always-on under existing gateway loop | Dedicated `serve` subcommand (clearest mental model) |
| D8 | Tokio runtime itself | ✅ **Resolved (§3.5):** keep — load-bearing across all 6 crates; concern was the *library*, not the runtime |

---

## 9. Risks

- **Unattended autonomy is a trust surface.** The §4.4 approval policy is non-negotiable — a proactive agent that can silently run `Dangerous` tools is a liability, not a feature. This gates the whole thesis.
- **Daemon reliability.** An always-on process needs supervision (systemd / launchd / Docker restart). Out of scope for the binary; documented for the user.
- **Clock/timezone/DST bugs** are classic cron footguns — lean on a maintained crate, test around DST boundaries.
- **Scope creep into Phase 6/7.** Layers 2–3 are explicitly *after* Layer 1. Do not let WASM or remote-agent design block the scheduler.
