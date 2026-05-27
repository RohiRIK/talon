# Talon — Implementation Roadmap

> **Generated:** 2026-05-26
> **Source:** PLAN.md phases, ordered by dependency chain
> **Principle:** Each phase unlocks the next. No phase starts until its dependencies' exit gates pass.

---

## Dependency Graph

```
Phase 0 (Foundation)
    │
    ▼
Phase 0.5 (Working Prototype)
    │
    ▼
Phase 1 (Core Agent Loop)
    │
    ├──────────────────┐
    ▼                  ▼
Phase 2 (Memory)    Phase 3 (Tools Tier 1)
    │                  │
    ▼                  │
Phase 2.5 (Iris)       │
    │                  │
    ├──────────────────┘
    ▼
Phase 4 (Gateway)
    │
    ▼
Phase 5 (Tools Tier 2 + MCP)
    │
    ▼
Phase 6 (Plugins + Scheduling)
    │
    ▼
Phase 7 (Advanced: Subagents, ACP, Evolution)
    │
    ▼
v1.0 Release
```

---

## Week-by-Week Timeline

### Week 1 — Skeleton & Proof of Life

| Phase | Tasks | Exit Gate |
|-------|-------|-----------|
| **Phase 0** | Workspace, Cargo.toml, CI, Docker, install.sh | `cargo build --workspace --release` green, CI green |
| **Phase 0.5** | EchoTool, AnthropicProvider, inline agent loop | `cargo run -- --message "read Cargo.toml"` works E2E |

**Unlocks:** All subsequent phases. The 7 load-bearing types get locked here.

### Weeks 2–3 — The Agent Thinks

| Phase | Tasks | Exit Gate |
|-------|-------|-----------|
| **Phase 1** | Core loop, LlmProvider trait, Tool trait, ApprovalMembrane, state machine, minimal DB | Agent responds to real LLM, messages persisted to SQLite |

**Unlocks:** Phase 2 (memory needs DB schema), Phase 3 (tools need dispatcher).

### Weeks 3–4 — The Agent Remembers

| Phase | Tasks | Exit Gate |
|-------|-------|-----------|
| **Phase 2** | Full SQLite schema, FTS5, MemoryStore trait, ContextBuilder, session_search tool | FTS5 search <50ms, context within token budget |

**Unlocks:** Phase 2.5 (Iris patterns need MemoryStore trait to exist).

### Weeks 4–5 — The Agent Remembers *Intelligently* (parallel with Phase 3)

| Phase | Tasks | Exit Gate |
|-------|-------|-----------|
| **Phase 2.5** | Two-tier memory, fact extraction, semantic dedup, hybrid search, semantic cache, Redis backend | Auto fact recall across sessions, cache hit on repeated prompts |
| **Phase 3** *(parallel)* | ReadFile, WriteFile, EditFile, Glob, Grep, Terminal+Docker sandbox, seccomp | `rm -rf /` blocked, all file tools functional |

**Why parallel:** Phase 2.5 (memory) and Phase 3 (tools) are independent — memory extends the MemoryStore trait, tools extend the Tool trait. No cross-dependency.

### Weeks 5–6 — The Agent Talks Everywhere

| Phase | Tasks | Exit Gate |
|-------|-------|-----------|
| **Phase 4** | Gateway trait, CLI, HTTP (Axum), TUI (Ratatui), Telegram (teloxide) | All 4 gateways functional, unified session memory |

**Depends on:** Phase 1 (agent loop), Phase 2 (session persistence). Does NOT depend on Phase 2.5 or 3.

### Weeks 6–7 — The Agent Reaches Out

| Phase | Tasks | Exit Gate |
|-------|-------|-----------|
| **Phase 5** | WebSearch, WebExtract, stdio subprocess plugins, MCP client+adapter, Browser (CDP) | MCP server tools discoverable, web search works |

**Depends on:** Phase 3 (tool infrastructure), Phase 4 (gateway for testing).

### Weeks 7–8 — The Agent Extends Itself

| Phase | Tasks | Exit Gate |
|-------|-------|-----------|
| **Phase 6** | WASM plugin host (wasmtime), SkillStore + hot-reload, CronScheduler, CronJobTool | WASM plugin loads without restart, cron fires on schedule |

**Depends on:** Phase 5 (subprocess plugin protocol validates the abstraction first).

### Weeks 8+ — The Agent Evolves

| Phase | Tasks | Exit Gate |
|-------|-------|-----------|
| **Phase 7** | Parallel subagents (JoinSet), ACP protocol, semantic search (fastembed), Discord, GEPA evolution sidecar, release pipeline | 3+ parallel subagents, `cargo dist` produces all-platform binaries |

**Depends on:** Everything. This is the capstone.

---

## Critical Path (Longest Chain)

The critical path determines the minimum time to v1.0:

```
Phase 0 → 0.5 → 1 → 2 → 2.5 → (wait for Phase 3) → 4 → 5 → 6 → 7
  1wk      1wk   2wk  1wk  1wk         1wk           1wk  1wk  1wk  2wk+
```

**Minimum: ~12 weeks** to feature-complete v1.0, assuming no blockers.

**Parallelism opportunities:**
- Phase 2.5 + Phase 3 (weeks 4–5)
- Phase 4 can start as soon as Phase 1 + 2 are done (doesn't need 2.5 or 3)

---

## Priority Stack (What to Build If Time Is Short)

If you can only ship N phases, this is the order of value:

1. **Phase 0 + 0.5 + 1** — Minimum viable agent (talks to LLM, runs tools)
2. **Phase 2** — Memory makes it useful (FTS5 search, session persistence)
3. **Phase 4** — Gateway makes it accessible (Telegram + CLI)
4. **Phase 3** — Tools make it powerful (file ops, terminal)
5. **Phase 2.5** — Iris memory makes it *intelligent* (auto-facts, semantic search, cache)
6. **Phase 5** — Web + MCP extends reach
7. **Phase 6** — Plugins + cron extend autonomy
8. **Phase 7** — Subagents + evolution extend capability

**The MVP is Phases 0–2 + 4:** An agent that talks, remembers, and is reachable via Telegram. Everything else is additive.

---

## Risk-Ordered Concerns

| Risk | Phase | Mitigation | Impact if Hit |
|------|-------|-----------|---------------|
| `async fn in trait` object safety | 1 | `Pin<Box<dyn Future>>` return, concrete enum dispatch | Blocks all phases |
| `rusqlite::Connection` is `!Send` | 1, 2 | `spawn_blocking` everywhere, `deadpool-sqlite` pool | Blocks memory |
| WASM ABI stability | 6 | Ship subprocess plugins first (Phase 5), WASM second | Delays plugins only |
| fastembed binary size (+30-60MB) | 2.5, 7 | Feature-flag `semantic-search`, track with `cargo bloat` | Cosmetic |
| Redis availability | 2.5 | Graceful fallback to SQLite, never crash | Degrades memory perf |
| LLM cost for fact extraction | 2.5 | Semantic cache, batch extraction per session | Higher operating cost |
| chromiumoxide/axum dep conflict | 5 | Use `headless_chrome` crate instead | Blocks browser tool |
| edition 2024 crate compatibility | 0 | Pin toolchain, track lagging crates | May need workarounds |
