# Incremental Migration Approach

> **Status:** ✅ Complete
> **Category:** Migration Strategy

---

## 1. The Core Problem

Rewriting everything at once is a trap. The Talon migration spans:
- 24 tool implementations (OpenClaw TS)
- 50+ tool implementations (Hermes Agent Python)
- SQLite schema migration
- 3+ gateway integrations
- Optional [self-evolution](../04_Core_Features/39_Self_Evolution_Loop.md) system

A big-bang rewrite means months before anything runs.
The incremental approach means **something useful runs in week 1**.

---

## 2. Migration Strategy: Strangler Fig

Talon uses the **Strangler Fig pattern**:
1. New Rust code wraps/replaces individual pieces
2. Original TS/Python code runs alongside during transition
3. Once a piece is migrated and tested, the old code is deleted
4. Eventually the old system is completely replaced

```
Week 1:   [  Rust core  ] + [mock tools]          → agent loop runs
Week 4:   [  Rust core  ] + [10 real tools]        → basic tasks work
Week 8:   [  Rust core  ] + [all tools]            → feature parity
Week 12:  [  Rust core  ] + [gateways]             → production ready
Week 20:  [  Rust core  ] + [self-evolution]       → Phase 2 complete
```

---

## 3. Phase-by-Phase Breakdown

### Phase 0: Foundation (Week 1–2)
**Goal:** Cargo workspace builds, minimal agent loop runs.

```
talon-core/
  ├── AgentLoop (hardcoded system prompt)
  ├── Message types
  ├── AnthropicProvider (streaming)
  └── 3 mock tools: echo, fail, sleep

talon-memory/
  └── SQLite schema + migrations (rusqlite)

talon-llm/
  └── LlmProvider trait + Anthropic impl

Tests: unit tests for message serialization, DB migrations
```

**Exit criteria:** `cargo run -- chat "Hello"` gets a response.

---

### Phase 1: Core Tools (Week 3–5)
**Goal:** Enough tools to be useful for basic development tasks.

Priority order (highest value / lowest risk first):

| Tool | Risk | Value | Week |
|------|------|-------|------|
| `read_file` | Low | High | 3 |
| `write_file` | Low | High | 3 |
| `search_files` | Low | High | 3 |
| `terminal_execute` | Med | High | 4 |
| `web_search` | Low | Med | 4 |
| `web_extract` | Low | Med | 4 |
| `patch` | Med | High | 5 |
| `todo` | Low | Med | 5 |
| `memory` | Low | High | 5 |

**Exit criteria:** Talon can complete a real coding task end-to-end.

---

### Phase 2: Memory & Sessions (Week 6–7)
**Goal:** Full session persistence and recall.

```
talon-memory/
  ├── Sessions table + FTS5 virtual table
  ├── session_search (three-shape API)
  ├── Memory entries (key/value notes)
  ├── Skill file system (read/write/list)
  └── User profile file

Tests: search correctness, scroll pagination, bookend context
```

**Exit criteria:** `session_search query="docker"` returns relevant past sessions.

---

### Phase 3: Gateway — CLI (Week 8)
**Goal:** Real TUI interaction, not just cargo run.

```
talon-gateway/
  └── cli/
      ├── ratatui TUI
      ├── Input handling + streaming output
      ├── Approval prompts (inline)
      └── Tool output formatting
```

**Exit criteria:** Full TUI chat session with streaming output.

---

### Phase 4: Gateway — Telegram (Week 9–10)
**Goal:** Telegram bot operational.

```
talon-gateway/
  └── telegram/
      ├── teloxide bot setup
      ├── Message handling
      ├── Media delivery (photos, audio, video)
      ├── Inline approval flow
      └── Cron delivery routing
```

**Exit criteria:** Talon responds to Telegram messages with full tool use.

---

### Phase 5: Scheduling (Week 11–12)
**Goal:** Cron jobs persist across restarts.

```
talon-core/
  └── cron/
      ├── CronJob SQLite schema
      ├── tokio-cron-scheduler integration
      ├── Job CRUD via cronjob tool
      └── Delivery routing (origin/local/all/platform:id)

Tests: schedule persistence, missed-run detection, delivery targeting
```

**Exit criteria:** A [cron job](../04_Core_Features/33_Cron_Scheduler.md) created on Monday still runs on Thursday after restart.

---

### Phase 6: Advanced Features (Week 13–16)
Parallelizable — can be done by separate workstreams:

| Feature | Crate | Notes |
|---------|-------|-------|
| [Browser tool](../04_Core_Features/32_Browser_Tool.md) | `talon-tools` | chromiumoxide |
| HTTP gateway | `talon-gateway` | axum |
| Discord gateway | `talon-gateway` | serenity |
| [Subagent delegation](../04_Core_Features/37_Subagent_Delegation.md) | `talon-core` | JoinSet |
| WASM plugins | `talon-plugins` | wasmtime |
| [Semantic search](../07_Memory_System/59_Embedding_Retrieval.md) | `talon-memory` | fastembed-rs (feature flag) |
| MCP client | `talon-tools` | rmcp |

---

### Phase 7: Self-Evolution (Week 17–20)
**Goal:** Talon can generate new skills from trajectories.

This is Phase 2 of the Talon roadmap — separate from the initial migration.
Architecture details in `docs/04_Core_Features/39_Self_Evolution_Loop.md`.

---

## 4. Testing Gates

Each phase must pass its tests before the next phase begins.

```bash
# Phase 0 gate
cargo test -p talon-core --lib
cargo test -p talon-llm --lib

# Phase 1 gate
cargo test -p talon-tools -- terminal file web

# Phase 2 gate
cargo test -p talon-memory -- session search skill

# Full regression
cargo test --workspace
```

---

## 5. Parallel Workstreams

After Phase 2, multiple people can work in parallel:

```
Stream A: CLI TUI (Phase 3)
Stream B: Telegram gateway (Phase 4)
Stream C: Browser tool (Phase 6 subset)
Stream D: WASM plugin system (Phase 6 subset)
```

Each stream only needs `talon-core` and `talon-memory` as stable foundations.

---

## 6. Feature Flag Strategy

New features land behind feature flags until stable:

```toml
[features]
default = ["telegram", "cli"]
http-gateway = ["dep:axum", "dep:tower"]
discord = ["dep:serenity"]
embeddings = ["dep:fastembed", "dep:sqlite-vec"]
wasm-plugins = ["dep:wasmtime"]
self-evolution = []    # logic only, no extra deps
```

This keeps the default binary small and compile times fast.

---

## 7. Rollback Plan

Each phase has a rollback: since Python/TS code runs in parallel during
migration, rolling back means pointing back to the old system.

The migration is complete when:
1. Talon handles 100% of production traffic
2. All tests pass
3. Old TS/Python code has been deleted
4. No feature regression reported in 2 weeks

**Final deletion commit message:**
`chore: delete Python/TS reference implementations — Talon is the system now`
---

## Related Documents

### Depends On
- [Migration Roadmap](21_Migration_Roadmap.md)

### See Also
- [Risk Register](28_Risk_Register.md)
- [Phase Build Guide](../00_Connections/06_Phase_Build_Guide.md)

