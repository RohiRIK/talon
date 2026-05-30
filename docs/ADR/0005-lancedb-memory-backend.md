# ADR 0005 — LanceDB as Memory Storage Backend

**Status:** ⚠️ Superseded by [ADR 0008 — SQLite + sqlite-vec](0008-sqlite-vec-memory-backend.md) (2026-05-30)
**Date:** 2026-05-27

## Context

Talon requires a persistent, queryable memory store that supports:
- Vector similarity search (semantic recall)
- Full-text search (FTS5-style keyword search)
- Hybrid retrieval (vector + FTS fused with Reciprocal Rank Fusion)
- No external server process (single-binary constraint)
- No cloud dependency

Candidates evaluated: Redis Iris (Redis Stack), pgvector (PostgreSQL), sqlite-vec, LanceDB.

## Decision

**LanceDB** is the memory storage engine for Talon LTM (long-term memory).

SQLite remains for operational data: sessions, messages, config, task queues, cron schedules.

These are two distinct databases with non-overlapping concerns:
- LanceDB: *what the agent knows* (memories, facts, embeddings)
- SQLite: *how the agent operates* (session state, config, coordination)

Redis is explicitly NOT a dependency. The Redis Iris patterns (two-tier memory, fact extraction, semantic dedup, hybrid search, semantic cache) are implemented in pure Rust via Talon LTM + LanceDB.

## Why LanceDB

| Criterion | LanceDB | pgvector | sqlite-vec | Redis Stack |
|-----------|---------|----------|------------|-------------|
| Embedded (no server) | ✅ | ❌ | ✅ | ❌ |
| Vector KNN search | ✅ | ✅ | ✅ | ✅ |
| Built-in FTS | ✅ | ❌ | ❌ | ✅ |
| Hybrid search + RRF | ✅ | Manual | Manual | Manual |
| Rust-first API | ✅ | Via sqlx | Via C FFI | Via redis-rs |
| License | Apache 2.0 | PostgreSQL | MIT | SSPL (non-free) |
| Disk I/O model | Arrow/Parquet | WAL | SQLite pages | In-memory |

## Consequences

**Positive:**
- Native Rust client, no Python bridge
- Single binary — LanceDB is embedded, no separate process
- Hybrid search built-in (LanceDB provides vector ANN + FTS, fused via RRF)
- Free license (Apache 2.0)

**Negative / Watch:**
- LanceDB Rust API is pre-1.0 (v0.9) — pin version, test carefully on upgrade
- Embedding model (`fastembed`) adds 30–60 MB to binary when `semantic-search` feature is enabled — keep behind feature flag
- LLM-powered fact extraction has a per-session cost — mitigated by semantic cache and batching (once per session end, not per turn)
