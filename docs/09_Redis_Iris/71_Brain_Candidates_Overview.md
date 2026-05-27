# 71 — Brain Candidates Overview

> **Status:** Research / Exploration  
> **Goal:** Evaluate memory/context engine ("Brain") candidates for Talon  
> **Context:** Successor to Redis Iris research (doc 66). Iris is Python-only, pip-distributed, no self-hosted option confirmed. We need alternatives — preferably Rust-native.

---

## The Problem

Talon needs a **Brain** — the memory/context layer that gives the agent:
1. **Working memory** — current conversation, tool results, scratchpad
2. **Long-term memory** — facts, preferences, decisions, patterns across sessions
3. **Semantic recall** — find relevant memories by meaning, not just keywords
4. **Decay & promotion** — stale memories fade, confirmed ones persist

The current plan (Phase 2.5) uses **SQLite + FTS5** with optional Redis behind a feature flag. This doc evaluates whether better options exist.

---

## Candidates

| # | Candidate | Type | Language | Embedded? | License |
|---|-----------|------|----------|-----------|---------|
| 1 | **claude-ltm-plugin** | Agent memory system | TypeScript/Bun | Yes (SQLite) | MIT |
| 2 | **LanceDB** | Vector + FTS database | Rust core | Yes | Apache 2.0 |
| 3 | **mem0-rust** | Agent memory layer | Rust | Yes (multi-backend) | MIT |
| 4 | **Qdrant** | Vector search engine | Rust | No (client-server) | Apache 2.0 |
| 5 | **Rig** | LLM agent framework | Rust | N/A (framework) | MIT |
| 6 | **Swiftide** | RAG/agent framework | Rust | N/A (framework) | MIT |

**Detailed analysis:** docs 72–75.

---

## Quick Comparison Matrix

| Feature | claude-ltm | LanceDB | mem0-rust | Qdrant |
|---------|-----------|---------|-----------|--------|
| Rust-native | ✗ (TS) | ✓ | ✓ | ✓ |
| Embedded (no server) | ✓ | ✓ | ✓ | ✗ |
| Vector search | ✓ (fallback) | ✓ | ✓ | ✓ |
| Full-text search | ✓ (FTS5) | ✓ | Varies | ✗ |
| Memory decay | ✓ | ✗ | ✗ | ✗ |
| Auto-extraction | ✓ | ✗ | ✓ | ✗ |
| Memory graphs | ✓ | ✗ | ✗ | ✗ |
| Maturity | Early (v2.1) | Growing (v0.29) | Very early | Mature (v1.18) |
| Stars | ~0 (new) | ~5k | ~few | ~31k |

---

## Recommendation Preview

**Architecture pattern to adopt:**
- **claude-ltm's memory model** (decay, importance, categories, graphs, auto-extraction) as the **design blueprint**
- **LanceDB** as the **storage engine** (replaces SQLite+FTS5, adds vector search natively)
- **mem0-rust patterns** for the **memory extraction/recall API**

This gives Talon: Rust-native embedded storage with vector+FTS, a proven memory model design, and zero external dependencies.

See individual candidate docs for full analysis.
