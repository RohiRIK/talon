# Redis Iris — Overview & Strategic Fit for Talon

> **Status:** ✅ Done
> **Category:** 09_Redis_Iris

---

## What Is Redis Iris?

Redis Iris is **not** an AI agent framework. It is a **unified, real-time context engine** — infrastructure that sits *beneath* agent frameworks to provide memory, data retrieval, and caching. It does not handle LLM orchestration, tool dispatch, reasoning loops, or agent logic.

Redis branded Iris as a product suite composed of 5 services:

| Service | What It Does | Open Source? |
|---------|-------------|-------------|
| **Agent Memory** | Two-tier memory: working (session) + long-term (semantic) | ✅ Apache 2.0 |
| **Context Retriever** | Schema-first MCP tool generation for business data | ❌ Redis Cloud only |
| **Redis Data Integration (RDI)** | CDC pipeline from Postgres/MySQL → Redis | ❌ Managed service |
| **LangCache** | Semantic caching of LLM responses | ❌ Managed service |
| **Redis Search** | Vector + full-text + hybrid search | ✅ (part of Redis) |

**Key insight for Talon:** Only Agent Memory and Redis Search are open-source and relevant. The rest are managed cloud services that conflict with Talon's self-hosted, single-binary philosophy.

---

## Why Consider Iris for Talon?

### The Problem Iris Solves

Iris's thesis: **"Context is all you need."** Agents fail not because of bad models, but because of fragmented, stale data. Memory should compound over time. This aligns perfectly with Talon's killer differentiator — persistent, queryable, cross-project memory.

### What Talon Gets from Iris's Architecture

1. **Two-tier memory model** — Session-scoped working memory + cross-session long-term recall. Talon currently plans SQLite+FTS5 for this. Redis offers sub-millisecond reads.
2. **Semantic search** — Vector similarity over memories, not just keyword FTS5. Redis Search supports vector, full-text, and hybrid in one engine.
3. **Real-time data freshness** — CDC pipelines keep agent context current with external systems.
4. **LLM response caching** — Semantic deduplication of similar prompts saves cost and latency.
5. **MCP tool generation** — Auto-generates tools from data schemas (Context Retriever pattern).

### What Talon Still Needs Independently

Iris provides **zero** of:
- Agent orchestration / reasoning loop
- Tool dispatch and execution
- Multi-agent coordination
- LLM provider abstraction
- Gateway / multi-channel support
- Approval membrane / security model

**Iris is a brain's memory system, not the brain itself.** Talon's `talon-core` agent loop remains untouched.

---

## Strategic Options

### Option A: Redis as Primary Memory Backend (Recommended)

Replace SQLite+FTS5 with Redis for the hot path. Keep SQLite for durable persistence (WAL backup).

```
┌─────────────────────────────────────────────┐
│                  Talon Agent                 │
│  ┌─────────┐  ┌──────────┐  ┌────────────┐ │
│  │  Core    │  │   LLM    │  │   Tools    │ │
│  │  Loop    │  │ Provider │  │            │ │
│  └────┬─────┘  └──────────┘  └────────────┘ │
│       │                                      │
│  ┌────▼─────────────────────────────────┐   │
│  │        talon-memory (hybrid)         │   │
│  │  ┌─────────┐     ┌───────────────┐   │   │
│  │  │  Redis  │◄───►│   SQLite      │   │   │
│  │  │ (hot)   │     │  (durable)    │   │   │
│  │  │ search  │     │  WAL backup   │   │   │
│  │  │ vectors │     │  migrations   │   │   │
│  │  │ cache   │     │  FTS5 fallback│   │   │
│  │  └─────────┘     └───────────────┘   │   │
│  └──────────────────────────────────────┘   │
└─────────────────────────────────────────────┘
```

**Pros:** Sub-ms memory reads, native vector search, semantic caching, proven at scale.
**Cons:** Adds Redis dependency (breaks "zero dependencies" story), needs Redis running.

### Option B: Adopt Iris Memory Patterns in Rust (No Redis Dependency)

Study the Agent Memory Server architecture and reimplement the two-tier memory model in pure Rust against SQLite. Use `fastembed` for vectors, FTS5 for text, custom semantic cache.

**Pros:** Single binary preserved, no external dependencies.
**Cons:** More implementation work, slower than Redis for hot reads.

### Option C: Redis as Optional Backend (Feature-Flagged)

Default to SQLite. When `feature = "redis-memory"` is enabled and Redis is available, use it as the primary store with SQLite as durable backup.

**Pros:** Best of both worlds. Single binary for simple use, Redis for power users.
**Cons:** Two code paths to maintain.

### Recommendation: Option C

This preserves Talon's "single binary, zero dependencies" story while offering Redis as a power-user upgrade path. The `MemoryStore` trait already abstracts the backend — adding a Redis implementation is additive, not disruptive.

---

## Related Documents

### Depends On
- [Context & Memory Architecture](../02_Architecture/15_Context_And_Memory_Architecture.md)
- [Memory System (SQLite + FTS5)](../04_Core_Features/35_Memory_System_SQLite_FTS5.md)

### See Also
- [Redis Iris Two-Tier Memory Model](67_Iris_Two_Tier_Memory.md)
- [Redis Iris Technical Integration](68_Iris_Technical_Integration.md)
- [Redis Iris Philosophy & Design](69_Iris_Philosophy.md)
- [Embedding-based Semantic Retrieval](../07_Memory_System/59_Embedding_Based_Retrieval.md)
