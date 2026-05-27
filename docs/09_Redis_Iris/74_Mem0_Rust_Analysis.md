# 74 — mem0-rust: Rust Port of the mem0 Memory Layer

> **Repo:** [YASSERRMD/mem0-rust](https://github.com/YASSERRMD/mem0-rust)  
> **Crate:** [mem0-rust](https://crates.io/crates/mem0-rust)  
> **Language:** Pure Rust  
> **License:** MIT  
> **Maturity:** Published Dec 2025, single maintainer, early stage

---

## What It Is

A Rust implementation of [mem0](https://github.com/mem0ai/mem0) — the "universal memory layer for AI agents." Provides automatic memory extraction from conversations, multi-backend storage, and semantic recall.

---

## Architecture

```
Conversation
    │
    ▼
┌─────────────────┐
│  Memory Manager  │
│  (LLM-powered)   │
│                   │
│  Extract → Store  │
│  Search → Recall  │
└─────────────────┘
    │         │
    ▼         ▼
┌────────┐ ┌────────────┐
│ Vector  │ │ Embedding   │
│ Store   │ │ Provider    │
│         │ │             │
│ Qdrant  │ │ OpenAI      │
│ PgVec   │ │ Ollama      │
│ Redis   │ │ HuggingFace │
│ Memory  │ │             │
└────────┘ └────────────┘
```

### Key Properties
- **Multi-backend vector stores:** In-memory, Qdrant, PostgreSQL (pgvector), Redis
- **Multi-embedding providers:** OpenAI, Ollama, HuggingFace
- **Multi-LLM providers:** OpenAI, Ollama, Anthropic
- **Automatic extraction:** LLM analyzes conversations and extracts memories
- **Semantic search:** Vector similarity for recall

---

## Rust API

```rust
use mem0_rust::{MemoryManager, MemoryConfig};

let config = MemoryConfig::default();
let manager = MemoryManager::new(config).await?;

// Add memories from a conversation
manager.add("The user prefers Rust over Python", "user_123").await?;

// Search memories
let results = manager.search("language preferences", "user_123").await?;

// Get all memories for a user
let memories = manager.get_all("user_123").await?;
```

---

## What's Good

1. **Closest to what Talon needs** — agent memory extraction and recall as a Rust crate
2. **Multi-backend** — swap storage without changing application code
3. **Automatic extraction** — LLM-powered memory extraction from conversations
4. **Pure Rust** — no FFI, no Python interop

---

## What's Concerning

1. **Single maintainer** — bus factor of 1
2. **Very early** — published Dec 2025, unclear production usage
3. **No memory decay** — memories persist forever (no importance/decay model)
4. **No memory relations** — flat list, no graph structure
5. **No FTS** — vector-only search (no keyword fallback)
6. **LLM-dependent** — requires an LLM call for every memory extraction
7. **API surface** — simpler than claude-ltm's model (no categories, no importance levels)

---

## Comparison: mem0-rust vs claude-ltm model

| Feature | mem0-rust | claude-ltm model |
|---------|-----------|-------------------|
| Auto-extraction | ✓ (LLM) | ✓ (LLM) |
| Categories | ✗ | ✓ (gotcha, arch, pref...) |
| Importance levels | ✗ | ✓ (1-5 stars) |
| Decay | ✗ | ✓ (time-based) |
| Memory graph | ✗ | ✓ (typed relations) |
| FTS search | ✗ | ✓ (FTS5) |
| Vector search | ✓ | ✓ (fallback) |
| Multi-backend | ✓ | ✗ (SQLite only) |
| Rust-native | ✓ | ✗ (TypeScript) |

---

## Integration Path for Talon

### Option A: Use as dependency
```toml
[dependencies]
mem0-rust = "0.x"
```
Direct dependency. Risk: single maintainer, limited features.

### Option B: Fork and extend
Fork mem0-rust, add missing features (decay, categories, relations, FTS). More control, more maintenance burden.

### Option C: Borrow patterns only
Study mem0-rust's multi-backend and extraction patterns. Implement Talon's own memory system with claude-ltm's richer model. **Recommended.**

---

## Verdict

★★★☆☆ — **Useful reference, not a dependency.** The multi-backend pattern and extraction API are worth studying, but the memory model is too thin compared to claude-ltm's. Talon should implement its own memory system inspired by both: claude-ltm's model + mem0-rust's backend abstraction.
