# Migration Roadmap & Phases

> **Status:** ✅ Complete
> **Category:** Migration Strategy

---

## 1. Philosophy: Parallel Build, Not Fork-and-Rewrite

Do NOT attempt to rewrite OpenClaw or Hermes line-by-line.
Build Talon from scratch in Rust, using the source repos as **specification documents** — what they do, not how they do it.

Keep OpenClaw/Hermes running in production while Talon is developed.
Switch over per-channel (start with CLI, then Telegram, then the rest).

---

## 2. Phase Overview

```
Phase 0 — Foundation     (Weeks 1–2)
Phase 1 — Core Loop      (Weeks 3–5)
Phase 2 — Memory         (Weeks 6–7)
Phase 3 — Tools (Tier 1) (Weeks 8–10)
Phase 4 — Gateways       (Weeks 11–13)
Phase 5 — Scheduler      (Week 14)
Phase 6 — Migration      (Weeks 15–16)
Phase 7 — Advanced       (Weeks 17–20)
```

---

## 3. Phase 0 — Foundation

**Goal:** Workspace compiles, CI runs, SQLite schema exists.

```
Tasks:
  ✅ Create Cargo workspace with all crate stubs
  ✅ Write workspace Cargo.toml with all deps pinned
  ✅ SQLite schema + migration runner (rusqlite)
  ✅ Config loading (config crate + TOML + env vars)
  ✅ Tracing setup (JSON + pretty modes)
  ✅ GitHub Actions CI (cargo check, clippy, test)
  ✅ Dockerfile (multi-stage, scratch final image)
```

**Exit criteria:** `cargo build` succeeds. `cargo test` passes (empty test suites). Docker image builds.

---

## 4. Phase 1 — Core Agent Loop

**Goal:** Single-turn LLM interaction works end-to-end from CLI.

```
Tasks:
  - LlmProvider trait + OpenAI-compatible client
  - Basic streaming SSE parser
  - CompletionRequest → Delta stream → print to stdout
  - Tool trait + ToolRegistry (empty)
  - ContextBuilder (static system prompt only)
  - CLI gateway (readline input, streamed output)
  - AgentState machine + SessionManager
  - AgentLimits (max_iter, token budget)
```

**Exit criteria:**
```bash
$ talon --model gpt-4o "What is 2+2?"
# Streams response to stdout. Session saved to SQLite.
```

---

## 5. Phase 2 — Memory

**Goal:** Sessions persist, context carries history, MEMORY.md injected.

```
Tasks:
  - MemoryStore full implementation
  - FTS5 virtual table + triggers
  - load_memory_md() / save_memory_md()
  - load_recent_messages() with token budget
  - SkillStore (filesystem scanner + in-memory cache)
  - skill_view / skills_list / skill_manage tools
  - memory tool (add / replace / remove)
  - session_search tool (FTS5 queries)
  - Skill hot-reload via notify watcher
  - AGENTS.md project context loading
```

**Exit criteria:**
```bash
$ talon "Remember my name is Rohi"
$ talon "What's my name?"
# Returns "Rohi" from MEMORY.md injection.
# FTS5 search finds the original message.
```

---

## 6. Phase 3 — Tools Tier 1

**Goal:** All read/write filesystem and web tools work.

```
Tools to implement:
  - terminal (tokio::process, timeout, background)
  - read_file / write_file / patch
  - search_files (rg subprocess)
  - web_search (reqwest + configurable backend)
  - web_extract (reqwest + scraper + comrak)
  - send_message (gateway dispatch)
  - Approval membrane (full implementation)
  - ToolContext with all fields populated
```

**Exit criteria:** Talon can complete a coding task: read a file, modify it, run tests, report results.

---

## 7. Phase 4 — Gateways

**Goal:** Telegram and Discord fully operational.

```
Tasks:
  - Gateway trait implementation
  - GatewayRouter (fan-out delivery)
  - CLI gateway (full TUI with ratatui)
  - Telegram gateway (teloxide, media, inline keyboards)
  - Discord gateway (serenity, slash commands)
  - HTTP gateway (axum, ACP protocol)
  - AgentEvent → gateway delivery mapping
  - Rate limiter per sender
  - Media upload helpers
```

**Exit criteria:** Rohi can chat with Talon on Telegram. Tool activity streams in real time.

---

## 8. Phase 5 — Scheduler

**Goal:** Cron jobs fully functional.

```
Tasks:
  - CronJob struct + CronStore
  - tokio-cron-scheduler integration
  - Human interval parser ("30m", "every 2h", "daily")
  - cronjob tool (create/list/update/pause/resume/remove/run)
  - Ephemeral agent sessions for cron
  - context_from: upstream job output injection
  - Deliver target routing
  - One-shot job auto-removal
  - Repeat count support
```

---

## 9. Phase 6 — Migration Cutover

**Goal:** Replace OpenClaw/Hermes with Talon for all active channels.

```
Tasks:
  - SQLite data migration script (import Hermes sessions)
  - MEMORY.md / USER.md / SKILL.md directory migration
  - Parallel run period (2 weeks both running, compare)
  - Per-channel cutover (CLI → Telegram → Discord)
  - Decommission OpenClaw/Hermes Docker containers
  - Update all cron jobs to target Talon
```

---

## 10. Phase 7 — Advanced Features

**Goal:** WASM plugins, [subagent delegation](../04_Core_Features/37_Subagent_Delegation.md), [self-evolution](../04_Core_Features/39_Self_Evolution_Loop.md) bridge.

```
Tasks:
  - talon-plugins crate (wasmtime host)
  - WasmTool wrapper + plugin ABI
  - Subagent delegation (tokio tasks + channels)
  - Self-evolution sidecar HTTP bridge
  - Trajectory collection (execution trace export)
  - Optional: fastembed-rs semantic search
  - Optional: voice mode (whisper-rs + rodio)
  - Optional: Prometheus metrics endpoint
```

---

## 11. Milestones Table

| Milestone | Target | Deliverable |
|-----------|--------|-------------|
| M0 | Week 2 | Workspace builds + CI green |
| M1 | Week 5 | CLI chat works end-to-end |
| M2 | Week 7 | Memory + skills persist |
| M3 | Week 10 | All Tier-1 tools pass integration tests |
| M4 | Week 13 | Telegram/Discord live |
| M5 | Week 14 | [Cron scheduler](../04_Core_Features/33_Cron_Scheduler.md) live |
| M6 | Week 16 | Full production cutover |
| M7 | Week 20 | WASM plugins + evolution bridge |
---

## Related Documents

### Depends On
- [Strategic Recommendations](../01_Analysis/10_Strategic_Recommendations.md)
- [System Architecture Overview](../02_Architecture/11_System_Architecture_Overview.md)

### See Also
- [Risk Register](28_Risk_Register.md)
- [Incremental Migration Approach](27_Incremental_Migration_Approach.md)
- [Phase Build Guide](../00_Connections/06_Phase_Build_Guide.md)

