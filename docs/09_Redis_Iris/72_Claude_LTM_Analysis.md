# 72 — claude-ltm-plugin: Rohi's Memory System

> **Repo:** [RohiRIK/claude-ltm-plugin](https://github.com/RohiRIK/claude-ltm-plugin)  
> **Language:** TypeScript (Bun runtime)  
> **Storage:** SQLite + FTS5 + optional vector embeddings  
> **License:** MIT  
> **Status:** v2.1.1, actively maintained by Rohi

---

## What It Is

A long-term memory plugin for Claude Code (and OpenCode, Pi). Gives AI coding agents persistent semantic memory that survives across sessions, updates, and context compaction.

**This is Rohi's own project** — the memory model is battle-tested from real use.

---

## Architecture

```
Session Start Hook
    │
    ▼
┌──────────────┐     ┌──────────────┐
│  Context      │────▶│  ltm.db      │
│  Injection    │     │  (SQLite)    │
└──────────────┘     │  - memories  │
                      │  - relations │
Session End Hook      │  - context   │
    │                 │  - projects  │
    ▼                 └──────────────┘
┌──────────────┐            │
│  Auto-Extract │           │
│  Patterns     │◀──────────┘
└──────────────┘
```

Key components:
- **MCP server** — tools for recall, learn, relate, forget
- **6 hooks** — auto-inject context at session start, auto-extract at session end
- **4 commands** — `/ltm:memory`, `/ltm:project`, `/ltm:health`, `/ltm:admin`
- **Graph visualizer** — Next.js app for browsing memory network (graph-app/)
- **892K lines TypeScript** across core + graph-app

---

## Memory Model (The Gold Standard)

This is the most complete agent memory model of all candidates:

### Categories
- `gotcha` — pitfalls and traps
- `architecture` — structural decisions
- `preference` — user/project conventions
- `pattern` — recurring approaches
- `decision` — explicit choices with rationale

### Importance (1–5 stars)
- ★ — trivial, decays fast
- ★★★ — useful, moderate lifetime
- ★★★★★ — permanent, never decays

### Decay
- Memories have a `decay_rate` that reduces relevance over time
- Unaccessed memories fade naturally
- Confirmed memories (accessed/validated) get decay reset
- `importance: 5` = permanent, immune to decay

### Relations (Memory Graph)
- Typed edges between memories: `supports`, `contradicts`, `refines`, `depends_on`
- Graph visualization with cluster detection
- Enables reasoning chains across related memories

### Auto-Extraction
- Session end hook reviews the conversation
- LLM extracts notable patterns, classifies them
- Stores with appropriate category + importance
- **Zero manual effort** — memory accumulates automatically

### Search Strategy
1. **FTS5 first** — fast keyword/phrase matching
2. **Vector fallback** — semantic similarity when FTS5 returns nothing
3. **Ranking** — relevance × importance × recency

---

## What Talon Should Steal

1. **The entire memory model** — categories, importance, decay, relations
2. **Auto-extraction pattern** — session-end hook that extracts learnings
3. **Context injection** — session-start hook that pre-loads relevant memories
4. **Decay as a feature** — time-based relevance with importance override
5. **Memory graph** — typed relations between memories for reasoning
6. **FTS5-first, vector-fallback** search strategy

---

## What Doesn't Apply

- **TypeScript/Bun stack** — Talon is Rust, needs reimplementation
- **Claude Code plugin system** — Talon has its own plugin architecture
- **MCP transport** — Talon uses native Rust traits, not MCP
- **Graph-app (Next.js)** — nice-to-have visualization, not core

---

## Integration Path for Talon

**Pattern adoption, not code adoption.** claude-ltm is the **design reference** — Talon implements the same memory model in Rust with a different storage backend.

```rust
// The memory model claude-ltm proves works:
struct Memory {
    id: Ulid,
    content: String,
    category: MemoryCategory,      // gotcha, architecture, preference...
    importance: u8,                 // 1-5, controls decay immunity
    decay_rate: f32,                // how fast it fades
    last_accessed: DateTime<Utc>,   // for decay calculation
    confirmed_count: u32,           // times validated/accessed
    tags: Vec<String>,
    project: Option<String>,
    embedding: Option<Vec<f32>>,    // for semantic search
}

struct MemoryRelation {
    source: Ulid,
    target: Ulid,
    relation_type: RelationType,   // supports, contradicts, refines...
}
```

**Verdict:** ★★★★★ as a design blueprint. The memory model is proven, well-thought-out, and maps cleanly to Rust structs. The TypeScript implementation is irrelevant — the *ideas* are what matter.
