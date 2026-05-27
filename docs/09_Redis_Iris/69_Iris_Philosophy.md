# Redis Iris — Philosophy & Design Principles

> **Status:** ✅ Done
> **Category:** 09_Redis_Iris

---

## Core Thesis: "Context Is All You Need"

Redis Iris is built on a single conviction: **agents fail because of bad context, not bad models.** The smartest LLM in the world will hallucinate if it doesn't have the right data at the right time. Iris exists to solve the data problem, not the reasoning problem.

This philosophy maps directly to Talon's own differentiator — persistent, queryable, cross-project memory. Where other agents treat memory as an afterthought, Talon and Iris both treat it as the foundation.

---

## Five Design Principles

### 1. Memory Should Compound

Every interaction should make the agent smarter. Not just for this session — forever. Iris achieves this through automatic fact extraction: every conversation produces structured knowledge that persists and gets deduplicated over time.

**Talon alignment:** This is exactly the `user_facts` + `MEMORY.md` pattern from Hermes, but automated. Talon should ship with automatic fact extraction from day one, not as an afterthought.

### 2. Data Should Be Connected, Not Siloed

Iris connects to databases via CDC (Change Data Capture), syncing business data into Redis in real-time. The agent doesn't query Postgres directly — it queries Redis, which mirrors Postgres with sub-second lag.

**Talon alignment:** While Talon won't ship CDC pipelines, the principle applies to memory. Sessions from Telegram, CLI, and Discord should all feed the same memory store. Cross-channel context is Talon's version of "connected data."

### 3. Speed Is a Feature

Redis exists because "fast enough" isn't fast enough. Sub-millisecond reads mean the agent's memory lookup never becomes the bottleneck. When you can query 100K vectors in 2ms, you can afford to check memory on every single turn.

**Talon alignment:** SQLite+FTS5 is fast (~5ms for keyword search). But vector search via fastembed is ~50ms. Redis Search does both in ~1-2ms. If Talon wants to query memory on every turn (which it should), Redis is the faster path.

### 4. Schema-First, Not Query-First

Iris's Context Retriever generates MCP tools from entity schemas. You define "a Customer has name, email, tier" and it auto-generates `get_customer`, `search_customers`, `list_customers` tools. The agent discovers what data exists through tool schemas, not raw SQL.

**Talon alignment:** This maps to Talon's tool system. Tools should be generated from data models, not hand-coded. The `MemoryStore` trait could expose its capabilities as discoverable tools — `search_memories`, `get_session`, `recall_fact` — generated from the schema.

### 5. Cache Semantically, Not Literally

LangCache doesn't cache exact string matches. It embeds the prompt and finds semantically similar cached responses. "What's the weather in NYC?" and "NYC weather?" hit the same cache entry.

**Talon alignment:** LLM calls are expensive (time and money). A semantic response cache in Talon would reduce costs and latency significantly for repeated patterns. This is especially valuable for cron jobs that ask similar questions on each run.

---

## Philosophical Tension: Single Binary vs. Infrastructure

The deepest tension between Talon and Iris is philosophical:

| | Talon | Iris |
|--|-------|------|
| **Identity** | Self-contained agent | Infrastructure layer |
| **Deployment** | Single binary, zero deps | Docker Compose (Redis + Python + app) |
| **Memory** | Embedded (SQLite) | External (Redis) |
| **Target user** | Solo developer, self-hosted | Enterprise, cloud-native |
| **Scaling unit** | Single machine | Distributed |

**Resolution:** Feature flags. The default Talon experience is the single binary with embedded SQLite. Power users who want Redis-tier performance opt into `feature = "redis-memory"`. The `MemoryStore` trait abstracts the backend — the agent loop doesn't know or care.

This is the same pattern Talon uses for semantic search (`feature = "semantic-search"` with fastembed). Redis memory is an additive capability, not a replacement.

---

## What Talon Should Steal

1. **Two-tier memory** — Working memory + long-term recall. Not optional. Core architecture.
2. **Automatic fact extraction** — Every conversation produces knowledge. LLM-powered.
3. **Semantic deduplication** — "User likes espresso" and "User enjoys espresso" = one fact.
4. **Hybrid search** — Vector + keyword with RRF fusion. Not one or the other.
5. **Semantic response cache** — Cache LLM responses by semantic similarity. Saves money.
6. **Memory promotion** — Important session facts get promoted to long-term automatically.

## What Talon Should NOT Steal

1. **CDC pipelines** — Talon is an agent, not a data integration platform.
2. **Managed cloud services** — Conflicts with self-hosted philosophy.
3. **Python server dependency** — No sidecar. Native Rust or nothing.
4. **Schema-first tool generation** — Interesting but premature for v1.

---

## Broader Landscape: How Others Handle Memory

| Agent | Memory Approach |
|-------|----------------|
| Claude Code | None. Stateless between sessions. |
| Hermes Agent | SQLite FTS5 + flat file (MEMORY.md, USER.md) + optional Mem0 |
| Aider | None. Zero persistence. |
| OpenClaw | None. |
| Goose | Basic session history, no cross-session. |
| **Talon (planned)** | **SQLite FTS5 + semantic vectors + optional Redis + two-tier + auto-extraction** |

Talon's memory system, informed by Iris, would be the most sophisticated open-source agent memory by a significant margin.

---

## Related Documents

### Depends On
- [Redis Iris Overview](66_Redis_Iris_Overview.md)

### See Also
- [Redis Iris Two-Tier Memory](67_Iris_Two_Tier_Memory.md)
- [Redis Iris Semantic Cache](70_Iris_Semantic_Cache.md)
- [Strategic Recommendations](../01_Analysis/10_Strategic_Recommendations.md)
