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
Phase 2.5 (Talon LTM) │
    │                  │
    ├──────────────────┘
    ▼
Phase 4 (Gateway)
    │
    ▼
Phase 5 (Tools Tier 2 + MCP)
    │
    ▼
v1.0 Release
    │
    ▼
Phase 6 (Plugins + Scheduling) ← v1.1
    │
    ▼
Phase 7 (Advanced: Subagents, ACP, Evolution) ← v2
```

---

## Week-by-Week Timeline

### Week 1 — Skeleton & Proof of Life

| Phase | Tasks | Exit Gate |
|-------|-------|-----------|
| **Phase 0** ✅ | Workspace, Cargo.toml, CI, Docker, versioning pipeline, install.sh | `cargo build --workspace --release` green, CI green, release workflow dry-run passes |
| **Phase 0.5** 👁 | EchoTool, AnthropicProvider, inline agent loop | `cargo run -- --message "read Cargo.toml"` works E2E |

**Phase 0 completed: 2026-05-27.** All 30 tasks done. Workspace (`edition="2024"`, 7 crates), CI workflow (SHA-pinned, 3-OS matrix), release workflow (OIDC, cosign, SLSA L2, cargo dist), `deny.toml` + `dependabot.yml` + `SECURITY.md`, `cliff.toml` + `dist-workspace.toml` + `install.sh`, ADRs 0001–0006, `CODEOWNERS` + `lefthook.yml`, full `main.rs` with `talon init`. Local exit gates passed: `cargo build --release`, `cargo nextest run`, `cargo clippy -D warnings`, `cargo audit`, `cargo deny check`, `docker build -t talon:0`. Rust 1.88 required (cargo-chef dependency floor). CI on GitHub and release dry-run (`v0.0.1-test` tag) pending first push.

> 👁 **First thing you see — end of Week 1:** `cargo run -- --message "read Cargo.toml"` prints a real LLM response to stdout. No UI yet — raw terminal output. One-shot, not interactive. But the agent is alive.

**Unlocks:** All subsequent phases. The 7 load-bearing types get locked here.

**Versioning is set up in Week 1, not Phase 7.** The full release pipeline (CI, signing, provenance, crates.io trusted publishing, cargo dist, Homebrew) is scaffolded in Phase 0. Every subsequent phase ships green against it. See `PLAN.md` Versioning Strategy section and Phase 0 tasks 0.9–0.24.

### Weeks 2–3 — The Agent Thinks

| Phase | Tasks | Exit Gate |
|-------|-------|-----------|
| **Phase 1** 👁 | Core loop, LlmProvider trait, Tool trait, ApprovalMembrane, state machine, minimal DB | Agent responds to real LLM, messages persisted to SQLite |

> 👁 **First real agent response — Weeks 2–3:** `cargo run --release -- --message "hello"` now goes through the full typed agent loop: LLM → tool dispatch → approval → response. AgentEvents stream to stdout. Still one-shot CLI, not interactive — but every message is persisted to SQLite and the approval membrane is live.

**Unlocks:** Phase 2 (memory needs DB schema), Phase 3 (tools need dispatcher).

### Weeks 3–4 — The Agent Remembers

| Phase | Tasks | Exit Gate |
|-------|-------|-----------|
| **Phase 2** | Full SQLite schema, FTS5, MemoryStore trait, ContextBuilder, session_search tool | FTS5 search <50ms, context within token budget |

**Unlocks:** Phase 2.5 (Iris patterns need MemoryStore trait to exist).

### Weeks 4–5 — The Agent Remembers *Intelligently* (parallel with Phase 3)

| Phase | Tasks | Exit Gate |
|-------|-------|-----------|
| **Phase 2.5** | talon-ltm memory model, LanceDB storage, fact extraction, semantic dedup, hybrid search, semantic cache | Auto fact recall across sessions, cache hit on repeated prompts |
| **Phase 3** *(parallel)* | ReadFile, WriteFile, EditFile, Glob, Grep, Terminal+Docker sandbox, seccomp | `rm -rf /` blocked, all file tools functional |

**Why parallel:** Phase 2.5 (memory) and Phase 3 (tools) are independent — memory extends the MemoryStore trait, tools extend the Tool trait. No cross-dependency.

**Memory stack decision:** LanceDB is the embedded memory storage engine (vectors + FTS + hybrid search). Redis is NOT a dependency. See `CLAUDE.md` for the full architecture decision.

### Weeks 5–6 — The Agent Talks Everywhere 🖥

| Phase | Tasks | Exit Gate |
|-------|-------|-----------|
| **Phase 4** 🖥 | Gateway trait, CLI, HTTP (Axum), TUI (Ratatui), Telegram (teloxide) | All 4 gateways functional, unified session memory |

> 🖥 **First interactive CLI — Week 5:** `cargo run --release -- --gateway cli` opens a persistent stdin/stdout chat loop. Type a message, get a response, keep going. Session memory is live — the agent remembers within the conversation.
>
> 🖥 **First TUI — Week 5–6:** `cargo run --release -- --gateway tui` opens a ratatui split-pane interface: input at the bottom, agent response stream at the top, AgentEvent status line (Thinking… / Calling tool… / Done). This is the first time Talon looks like an app.
>
> 📱 **First Telegram — Week 6:** Set `TELEGRAM_BOT_TOKEN`, send "hello" from your phone → response in under 5 seconds.

**Depends on:** Phase 1 (agent loop), Phase 2 (session persistence). Does NOT depend on Phase 2.5 or 3.

### Weeks 6–7 — The Agent Reaches Out

| Phase | Tasks | Exit Gate |
|-------|-------|-----------|
| **Phase 5** | WebSearch, WebExtract, stdio subprocess plugins, MCP client+adapter, Browser (CDP) | MCP server tools discoverable, web search works |

**Depends on:** Phase 3 (tool infrastructure), Phase 4 (gateway for testing).

### Weeks 7–8 — The Agent Extends Itself (v1.1)

| Phase | Tasks | Exit Gate |
|-------|-------|-----------|
| **Phase 6** | WASM plugin host (wasmtime), SkillStore + hot-reload, CronScheduler, CronJobTool | WASM plugin loads without restart, cron fires on schedule |

**Depends on:** Phase 5 (subprocess plugin protocol validates the abstraction first).

> **Scope note:** Phase 6 is v1.1, not v1.0. wasmtime + WASI preview2 is legitimately complex. The subprocess plugin protocol in Phase 5 gives immediate value and validates the abstraction. Ship that; WASM hot-reload follows.

### Weeks 8+ — The Agent Evolves (v2)

| Phase | Tasks | Exit Gate |
|-------|-------|-----------|
| **Phase 7** | Parallel subagents (JoinSet), ACP protocol, semantic search (fastembed), Discord, GEPA evolution sidecar, release pipeline | 3+ parallel subagents, `cargo dist` produces all-platform binaries |

**Depends on:** Everything. This is the capstone.

> **Scope note:** Phase 7 is v2, not v1.0. The evolution sidecar (GEPA/DSPy) is explicitly deferred. Do not let Phase 7 scope block calling v1.0 done.

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
