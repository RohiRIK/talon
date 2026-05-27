# Redis Iris — Two-Tier Memory Model

> **Status:** ✅ Done
> **Category:** 09_Redis_Iris

---

## Overview

The Agent Memory Server implements a two-tier memory architecture that maps directly to how humans remember: **working memory** (what's happening now) and **long-term memory** (what matters across time).

This is the single most valuable pattern Talon should adopt from Iris, regardless of whether Redis is the backend.

---

## Tier 1: Working Memory (Session-Scoped)

Working memory tracks the current conversation. It handles:

- **Message history** — Full conversation log for the active session
- **Window management** — Automatic trimming when context exceeds token budget
- **Summarization** — LLM-generated summaries of trimmed messages (preserves intent without tokens)
- **Session metadata** — User ID, session ID, timestamps, channel source

### How Iris Does It

```
Session Start
    │
    ▼
Messages accumulate in Redis List
    │
    ▼
Token count exceeds window? ──No──► Continue
    │
   Yes
    ▼
Summarize oldest N messages via LLM
    │
    ▼
Replace N messages with summary message
    │
    ▼
Continue (window stays within budget)
```

### Talon Adaptation

Talon's `ContextBuilder` (doc #15) already plans window management. The Iris pattern adds:

1. **Automatic summarization** — Don't just drop old messages; summarize them. The summary becomes part of the context.
2. **Summary chain** — When a summary itself gets old, it can be re-summarized. This creates a compression chain that preserves the most important context indefinitely.

```rust
// Proposed addition to talon-memory
pub struct WorkingMemory {
    messages: Vec<Message>,
    summaries: Vec<Summary>,
    token_budget: usize,
    window_size: usize,
}

impl WorkingMemory {
    /// When messages exceed window, summarize the oldest batch
    pub async fn compact(&mut self, llm: &dyn LlmProvider) -> Result<()> {
        if self.token_count() <= self.token_budget {
            return Ok(());
        }
        let oldest = self.messages.drain(..self.window_size / 2).collect::<Vec<_>>();
        let summary = llm.summarize(&oldest).await?;
        self.summaries.push(summary);
        Ok(())
    }
}
```

---

## Tier 2: Long-Term Memory (Cross-Session)

Long-term memory persists facts, preferences, and knowledge across sessions. This is where Iris's approach differs most from Talon's current FTS5-only plan.

### How Iris Does It

1. **LLM-powered extraction** — After each conversation turn, an LLM extracts structured facts: `("user prefers dark mode", topic="preferences", entities=["user", "dark mode"])`
2. **Deduplication** — New facts are compared against existing ones (semantic similarity). Duplicates are merged or updated, not appended.
3. **Vector storage** — Facts are embedded and stored as vectors in Redis Search for semantic retrieval.
4. **Hybrid search** — Queries use both vector similarity and keyword matching (RRF fusion).
5. **Metadata filtering** — Facts tagged with `user_id`, `session_id`, `topic`, `entities` for scoped retrieval.

```
Conversation Turn
    │
    ▼
LLM extracts facts ──► "User is a security architect"
    │                    "User prefers self-hosted solutions"
    ▼                    "Project uses Docker Swarm"
Embed each fact (vector)
    │
    ▼
Search existing facts (semantic similarity)
    │
    ├── Match found (>0.85 similarity)
    │       ▼
    │   Update existing fact (merge/replace)
    │
    └── No match
            ▼
        Insert new fact
```

### Talon Adaptation

This maps to Talon's `user_facts` table (Phase 2) and `EmbeddingStore` (Phase 7). The Iris pattern suggests:

1. **Automatic fact extraction** — Don't wait for explicit `memory save` commands. Extract facts from every conversation.
2. **Semantic deduplication** — Prevent memory bloat. "User likes dark mode" and "User prefers dark themes" should be one fact.
3. **Tiered retrieval** — First check working memory (fast, session-local), then long-term (semantic search).

```rust
pub struct LongTermMemory {
    store: Arc<dyn MemoryStore>,  // SQLite or Redis backend
    embedder: Arc<dyn Embedder>, // fastembed or API-based
}

impl LongTermMemory {
    /// Extract and store facts from a conversation turn
    pub async fn ingest(&self, messages: &[Message], llm: &dyn LlmProvider) -> Result<Vec<Fact>> {
        let facts = llm.extract_facts(messages).await?;
        let mut stored = Vec::new();
        for fact in facts {
            let embedding = self.embedder.embed(&fact.content).await?;
            let similar = self.store.search_similar(&embedding, 0.85).await?;
            if let Some(existing) = similar.first() {
                self.store.update_fact(existing.id, &fact).await?;
            } else {
                self.store.insert_fact(&fact, &embedding).await?;
            }
            stored.push(fact);
        }
        Ok(stored)
    }
}
```

---

## Memory Promotion: Working → Long-Term

The bridge between tiers is **promotion** — important facts from the current session get promoted to long-term storage. Iris uses LLM-based importance scoring.

Talon should implement this as a post-session hook:

```
Session Ends
    │
    ▼
Extract facts from full conversation
    │
    ▼
Score importance (LLM or heuristic)
    │
    ▼
Promote high-importance facts to long-term store
    │
    ▼
Deduplicate against existing long-term facts
```

---

## Impact on Talon Architecture

| Component | Current Plan | With Iris Pattern |
|-----------|-------------|------------------|
| `talon-memory` | SQLite + FTS5 only | SQLite + FTS5 + optional Redis + two-tier model |
| `ContextBuilder` | Static window | Dynamic window with auto-summarization |
| `user_facts` table | Manual save | Automatic LLM extraction + dedup |
| Semantic search | Feature-flagged fastembed | Core feature (Redis Search or fastembed) |
| Session persistence | All messages stored | Messages + summaries + promoted facts |

---

## Related Documents

### Depends On
- [Redis Iris Overview](66_Redis_Iris_Overview.md)
- [Context & Memory Architecture](../02_Architecture/15_Context_And_Memory_Architecture.md)

### See Also
- [Redis Iris Technical Integration](68_Iris_Technical_Integration.md)
- [Memory System (SQLite + FTS5)](../04_Core_Features/35_Memory_System_SQLite_FTS5.md)
- [Embedding-based Semantic Retrieval](../07_Memory_System/59_Embedding_Based_Retrieval.md)
