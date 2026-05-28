# 75 — Qdrant, Rig & Swiftide: Rust Ecosystem Players

> Three Rust projects that aren't direct brain replacements but provide important patterns and potential integration points for Talon's memory layer.

---

## 1. Qdrant — Rust-Native Vector Search Engine

**Repo:** [qdrant/qdrant](https://github.com/qdrant/qdrant)  
**Stars:** ~31.6k | **Version:** v1.18 | **License:** Apache 2.0

### What It Is
Production-grade vector similarity search engine. The most mature Rust AI project. Client-server architecture with gRPC/REST API.

### Architecture
- Collections of points (vector + JSON payload)
- HNSW indexing with configurable parameters
- Scalar, product, and binary quantization
- Rich filtering on payload fields
- Multi-vector and sparse vector support
- Distributed mode with sharding and replication

### Rust Client
```rust
use qdrant_client::Qdrant;

let client = Qdrant::from_url("http://localhost:6334").build()?;
client.upsert_points("memories", points).await?;
let results = client.search_points(SearchPoints {
    collection_name: "memories".into(),
    vector: embedding,
    limit: 10,
    filter: Some(Filter::must([Condition::matches("category", "gotcha")])),
    ..Default::default()
}).await?;
```

### For Talon
- **Pro:** Most mature, battle-tested, excellent Rust client
- **Con:** Requires a separate server process — breaks single-binary story
- **Role:** Optional backend behind feature flag for power users who want distributed vector search
- **Verdict:** ★★★☆☆ as brain component. Great DB, wrong deployment model for embedded agent.

---

## 2. Rig — Rust LLM Agent Framework

**Repo:** [0xPlaygrounds/rig](https://github.com/0xPlaygrounds/rig)  
**Stars:** Growing | **Version:** v0.37 | **License:** MIT  
**Website:** [rig.rs](https://rig.rs)

### What It Is
Modular LLM framework with unified provider interface, agent abstractions, vector store integrations, and RAG support. Used in production by Dria, Nethermind, Neon.

### Memory Model
```rust
// Rig's memory traits — worth studying
trait ConversationMemory {
    async fn store(&self, message: Message) -> Result<()>;
    async fn retrieve(&self, limit: usize) -> Result<Vec<Message>>;
}

// Built-in implementations:
// - InMemoryConversationMemory (volatile)
// - SlidingWindowMemory (token budget)

// VectorStore trait for RAG
trait VectorStoreIndex {
    async fn top_n(&self, query: &str, n: usize) -> Result<Vec<Document>>;
}
```

### Companion crate: `rig-memory`
- Sliding window policy
- Token budget management
- Pluggable backends

### For Talon
- **Pro:** Clean Rust trait abstractions for memory, good patterns to adopt
- **Con:** Framework-level dependency — too heavy, couples Talon to Rig's design
- **Role:** **Design reference** for trait design. Study `ConversationMemory`, `VectorStoreIndex`, and `rig-memory` patterns. Don't depend on it.
- **Verdict:** ★★★☆☆ — Excellent patterns, wrong coupling.

---

## 3. Swiftide — Rust RAG & Agent Framework

**Repo:** [bosun-ai/swiftide](https://github.com/bosun-ai/swiftide)  
**Version:** v0.32 | **License:** MIT  
**Website:** [swiftide.rs](https://swiftide.rs)

### What It Is
Streaming RAG pipeline framework. Handles ingestion, chunking, embedding, storage, and retrieval. Also has agent capabilities with tool calling.

### Architecture
```
Pipeline: Loader → Transformer → Chunker → Embedder → Store
                                                        │
                                          ┌─────────────┤
                                          │             │
                                       Qdrant      LanceDB
```

### Key Features
- Streaming async pipelines
- `#[tool]` macro for agent tool definitions
- Lifecycle hooks system
- OpenTelemetry instrumented
- Integrates with Qdrant, LanceDB, and other stores

### For Talon
- **Pro:** Pipeline design patterns, tool macro, hook system
- **Con:** Heavy framework with many dependencies, thin on persistent memory
- **Role:** **Inspiration for pipeline design.** The streaming indexing pipeline pattern could inform how Talon ingests and processes context. The `#[tool]` macro pattern is interesting.
- **Verdict:** ★★☆☆☆ — Too heavy as a dependency, interesting as inspiration.

---

## Summary: What to Borrow From Each

| Project | Borrow | Don't Borrow |
|---------|--------|-------------|
| **Qdrant** | Filtering API design, quantization strategies | Server architecture, gRPC transport |
| **Rig** | `ConversationMemory` trait, `VectorStoreIndex` trait, sliding window policy | Framework coupling, provider abstractions |
| **Swiftide** | Streaming pipeline pattern, `#[tool]` macro | Full framework, heavy dependency tree |
