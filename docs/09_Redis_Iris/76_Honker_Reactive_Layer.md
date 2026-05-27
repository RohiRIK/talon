# Doc 76 — Honker: SQLite Reactive Layer for Talon

> **Repository:** [russellromney/honker](https://github.com/russellromney/honker) · 1.9k ★ · 277 commits · Alpha
> **Role:** Reactive infrastructure — NOT a Brain candidate, but a **nervous system** that any Brain benefits from
> **Rating:** ★★★★☆ (strong fit as reactive plumbing layer)

---

## What Honker Is

A **Rust-native SQLite extension** that adds Postgres-style `NOTIFY`/`LISTEN` semantics to SQLite, with built-in:

- **Durable task queues** — retries, priority, delayed jobs, dead-letter, atomic with business writes
- **Streams** — per-consumer offsets, replay, durable pub/sub
- **Notify/Listen** — cross-process push notifications, single-digit ms latency
- **Scheduler** — 5/6-field cron + `@every` intervals, leader-elected, addressable schedules (pause/resume/update)

All in the **same `.db` file**. No Redis, no broker, no daemon. The core is `honker-core` (Rust crate) + `honker-extension` (loadable SQLite extension). Bindings exist for Python, Node, Bun, Go, Ruby, Elixir, .NET, JVM.

**Key insight:** `INSERT INTO orders` and `queue.enqueue(...)` commit in the **same transaction**. Rollback drops both. No dual-write problem.

---

## How Honker Connects with Talon's Memory (talon-ltm)

> **Important:** claude-ltm-plugin is a **design blueprint**, not a dependency. Talon builds its own
> memory system (`talon-ltm`) reimplemented in Rust, inspired by claude-ltm's architecture:
> categories, importance 1–5, decay, FTS5-first search, auto-extraction. The typed memory
> graph is optional — can be added later if needed, or skipped entirely.
> See [Doc 72](72_Claude_LTM_Analysis.md) for the full blueprint analysis.

Honker provides the **reactive plumbing** (when things happen) on top of talon-ltm. Four integration points:

### 1. Memory Change Notifications

When a memory is created, updated, or decayed → Honker `NOTIFY` pushes to tools, UI, other agents instantly. No polling loop required.

```rust
// Inside talon-memory crate
fn store_memory(db: &Connection, memory: &Memory) -> Result<()> {
    let tx = db.transaction()?;
    tx.execute("INSERT INTO memories ...", params![...])?;
    tx.notify("memory-changed", &json!({
        "id": memory.id,
        "category": memory.category,
        "importance": memory.importance,
    }))?;
    tx.commit()?; // atomic: memory write + notification
    Ok(())
}
```

### 2. Memory Maintenance as Queue Jobs

Decay recalculation, deduplication, memory promotion (working → long-term) — these are perfect queue tasks. Atomic with the triggering write:

```rust
// Promote important working memory to long-term
let maintenance = db.queue("memory-maintenance")?;
maintenance.enqueue(&json!({
    "task": "promote",
    "memory_id": id,
    "reason": "importance >= 4, accessed 5+ times"
}), Some(&tx))?; // same transaction as the access log update
```

### 3. Scheduled Maintenance

Honker's built-in cron scheduler handles periodic tasks without external cron:

- **Decay sweep** — `@every 1h`: recalculate importance scores based on time decay
- **Dedup pass** — `0 3 * * *`: nightly semantic deduplication across memory categories
- **Stale pruning** — `@every 6h`: remove memories below importance threshold
- **Stats rollup** — `0 0 * * 0`: weekly memory usage analytics

All leader-elected — safe with multiple Talon instances sharing one DB.

### 4. Multi-Agent Coordination

Multiple Talon agents on one `.db` file get **real-time coordination** through the memory layer:

```
Agent A stores fact → NOTIFY fires → Agent B wakes immediately
                                   → Agent C's stream consumer picks it up
                                   → Dashboard UI updates live
```

No polling. No message broker. Just SQLite commits triggering cross-process wakes in single-digit milliseconds.

---

## Why Honker Fits Talon Specifically

| Factor | Honker | Benefit for Talon |
|--------|--------|-------------------|
| **Language** | Rust (`honker-core` crate) | Same ecosystem, direct dependency |
| **Storage** | SQLite | Matches Talon's default, no new dependency |
| **Binary story** | Compiled-in extension | Single-binary preserved |
| **Transactions** | Business write + queue + notify = one commit | Memory consistency guaranteed |
| **Latency** | Single-digit ms cross-process | Near-real-time agent coordination |
| **Scheduler** | Built-in, leader-elected | Replaces external cron for memory maintenance |
| **Maturity** | Alpha, 1.9k ★, active development | Acceptable for a pre-v1.0 project like Talon |

---

## Architecture: Where Honker Sits

```
┌──────────────────────────────────────────────────┐
│                  Talon Agent                      │
│                                                   │
│  ┌─────────────┐  ┌──────────────┐  ┌──────────┐ │
│  │  talon-ltm   │  │   LanceDB    │  │  Honker  │ │
│  │ (own impl,   │  │  Vector Store │  │ Reactive │ │
│  │  claude-ltm  │  │              │  │  Layer   │ │
│  │  blueprint)  │  │ • embeddings │  │ • notify │ │
│  │ • categories │  │ • FTS        │  │ • queues │ │
│  │ • importance │  │ • hybrid     │  │ • streams│ │
│  │ • decay      │  │   search     │  │ • cron   │ │
│  │ • [graph?]   │  │              │  │          │ │
│  └──────┬───────┘  └──────┬───────┘  └────┬─────┘ │
│         │                 │               │       │
│         └────────┬────────┘               │       │
│                  │                        │       │
│         ┌────────▼────────────────────────▼──┐    │
│         │          SQLite (.db file)          │    │
│         │  memories + vectors + queues +      │    │
│         │  streams + schedules               │    │
│         └────────────────────────────────────┘    │
└──────────────────────────────────────────────────┘
```

**talon-ltm** = what memories look like (own Rust impl, claude-ltm blueprint)
**LanceDB** = where vectors live (storage)
**Honker** = when things react (events)
**Graph** = optional, add later if memory relationships prove valuable

---

## Integration Strategy for Talon

### Option A: Direct Dependency (Recommended)

Add `honker-core` as a Cargo dependency behind a feature flag:

```toml
[dependencies]
honker-core = { version = "0.x", optional = true }

[features]
default = ["sqlite-memory"]
reactive-memory = ["honker-core"]  # enables notify + queues + scheduler
```

### Option B: Loadable Extension

Load `honker` as a SQLite extension at runtime. Less tight integration but zero compile-time dependency:

```rust
conn.load_extension("honker")?;
conn.execute("SELECT honker_notify('channel', ?)", [payload])?;
```

### Recommendation

**Option A** for Talon — `honker-core` is a Rust crate, integrates naturally into the workspace. Feature-flagged so the minimal build stays lean.

---

## What Honker Does NOT Provide

- ❌ Memory model (categories, importance, decay) — that's claude-ltm
- ❌ Vector search / embeddings — that's LanceDB
- ❌ LLM fact extraction — that's the agent loop
- ❌ Task pipelines / DAGs / workflow orchestration — deliberately excluded
- ❌ Multi-writer replication — single-writer SQLite only

---

## Risks & Mitigations

| Risk | Severity | Mitigation |
|------|----------|------------|
| Alpha maturity | Medium | Talon is also pre-v1.0; both mature together |
| Single maintainer bus factor | Medium | Core is ~5k LOC Rust, forkable. SQLite schema is documented |
| PRAGMA data_version polling (1ms) | Low | Experimental SHM/kernel watchers available; default is proven |
| Feature velocity | Low | 277 commits, active PRs, responsive maintainer |

---

## Prior Art Comparison

| | Honker | Oban (Elixir) | pg-boss (Node) | Huey (Python) |
|---|--------|--------------|----------------|---------------|
| Database | SQLite | PostgreSQL | PostgreSQL | SQLite/Redis |
| Language | Rust | Elixir | JavaScript | Python |
| Embedded | ✅ | ❌ | ❌ | ✅ |
| Notify/Listen | ✅ | via pg_notify | via pg_notify | ❌ |
| Streams | ✅ | ❌ | ❌ | ❌ |
| Scheduler | ✅ | ✅ | ✅ | ✅ |
| Single-binary | ✅ | ❌ | ❌ | ❌ |

Honker is essentially "what if Oban/pg-boss ran on SQLite instead of Postgres" — which is exactly Talon's world.

---

## Conclusion

Honker is not a Brain candidate — it's the **nervous system** that makes the Brain reactive. Combined with talon-ltm (own Rust memory implementation, claude-ltm blueprint) and LanceDB (vector storage), it completes the trifecta:

- **talon-ltm** designs the memory (Rust reimplementation of claude-ltm's architecture — categories, importance, decay, FTS5. Graph layer optional/later)
- **LanceDB** stores and searches it (vectors + FTS)
- **Honker** makes it live — notifications, background jobs, scheduled maintenance, multi-agent coordination

All in one SQLite file, all in Rust, all preserving the single-binary story.
