# Concept Clusters — Talon Docs

> **Evidence source:** graphify community analysis (102 communities, 1,667 nodes)
> Communities with cohesion ≥ 0.14 are tight semantic clusters (self-contained, high internal coupling).
> Communities < 0.10 are broad buckets (cross-cutting concerns, many docs share loosely).

---

## Cluster A — Core Runtime

**Cohesion:** High (0.14–0.22 per community)
**Description:** The minimal viable Talon — what you build in Phase 0 and 1 of the migration. Everything else depends on this.

| Doc | Title | Community | Cohesion |
|-----|-------|-----------|----------|
| 11 | Talon System Architecture Overview | 97 | 0.22 |
| 12 | Cargo Workspace Design | 96 | 0.20 |
| 13 | Agent Loop Implementation | 58 | 0.14 |
| 14 | Tool System Architecture | 55 | 0.14 |
| 20 | State Machine & Agent Lifecycle | 68 | 0.14 |
| 54 | Error Handling Strategy | 78 | 0.15 |
| 50 | Tokio Runtime Configuration | 22 | 0.11 |
| 60 | Build System / Cargo Workspace | 51 | 0.13 |

**God Doc:** **Doc 13 — Agent Loop Implementation**
- Query `"agent loop state machine"` returns 33 nodes
- Community 58 is the tightest architecture cluster with the loop at center
- All other runtime docs feed into or are called from the loop

**Key shared concepts across cluster A:**
- `pub struct Agent` / `pub struct AgentConfig`
- `AgentEvent` enum
- `Arc<dyn Tool>` / `ToolRegistry`
- `LlmProvider` trait
- `tokio::spawn` / `JoinSet`
- `thiserror` / `anyhow` error hierarchy

**Build order within cluster:**
```
Doc 60 (Workspace) → Doc 12 (Design) → Doc 11 (Architecture)
                                      → Doc 50 (Tokio)
                                      → Doc 54 (Errors)
                                      → Doc 14 (Tool Trait)
                                      → Doc 20 (State Machine)
                                      → Doc 13 (Agent Loop) ← convergence point
```

---

## Cluster B — Tool Ecosystem

**Cohesion:** Medium-High (0.12–0.15)
**Description:** Everything tools. The trait contract, execution engine, approval safety layer, and all the individual tool implementations.

| Doc | Title | Community | Cohesion |
|-----|-------|-----------|----------|
| 14 | Tool System Architecture | 55 | 0.14 |
| 17 | Plugin & Skill Architecture + Approval Membrane | 31+56 | 0.12–0.14 |
| 29 | Terminal Tool | 41 | 0.12 |
| 30 | Tool Execution Engine | 74 | 0.15 |
| 32 | Browser Tool | 20 | 0.11 |
| 33 | Web Search & Extract | 44 | 0.13 |
| 34 | Send Message Tool | 32 | 0.12 |
| 36 | MCP Client Tool | 95 | 0.20 |
| 52 | Async Tool Execution | 83 | 0.17 |
| 59 | File System Tools | 59 | 0.14 |

**God Doc:** **Doc 30 — Tool Execution Engine**
- Query `"tool execution pipeline"` → 36 nodes centered on Community 74
- Bridges Doc 14 (trait) → Doc 17 (approval) → all tool impls
- Every tool call passes through this engine

**Key shared concepts:**
- `pub trait Tool: Send + Sync`
- `fn schema() -> Value` (schemars derive)
- `fn approval_level() -> ApprovalLevel`
- `async fn execute(ctx: ToolContext, params: Value) -> ToolResult`
- `ApprovalLevel::{Safe, NeedsApproval, Dangerous}`
- `ToolResult { output: String, is_error: bool, metadata: Option<Value> }`

**What binds this cluster:**
The Tool trait is the universal interface — every doc in this cluster either defines it (Doc 14), enforces safety on it (Doc 17), executes it (Doc 30, 52), or implements it (Docs 29, 32, 33, 34, 36, 59).

---

## Cluster C — Memory & Retrieval

**Cohesion:** Medium-High (0.12–0.15)
**Description:** All persistence. SQLite schema, FTS5 search, optional semantic search, session management, user model.

| Doc | Title | Community | Cohesion |
|-----|-------|-----------|----------|
| 15 | Context & Memory Architecture | 69 | 0.14 |
| 35 | Memory System (SQLite + FTS5) | 75 | 0.15 |
| 37 | Cron & Scheduling | 35 | 0.12 |
| 40 | Profile Isolation | 45 | 0.13 |
| 55 | SQLite + FTS5 in Rust | 38 | 0.12 |
| 56 | Embedding Retrieval | 67 | 0.14 |
| 57 | Session Search | 18 | 0.10 |
| 58 | FTS5 Deep Dive | 18 | 0.10 |
| 64 | Config System | 70+72 | 0.14–0.15 |
| 66 | User Model (USER.md) | 50 | 0.13 |

**God Doc:** **Doc 55 — SQLite + FTS5 in Rust**
- Query `"SQLite FTS5 retrieval"` → 35 nodes
- `graphify path "Memory System: SQLite + FTS5" "FTS5 Full-Text Search — Deep Dive"` found 2-hop connection via FTS5 Query Syntax — the only confirmed graph path in the entire query set
- All memory docs use the same SQLite connection pool and schema

**Key shared concepts:**
- `pub struct Database { pool: Arc<Mutex<Connection>> }`
- FTS5 virtual table: `CREATE VIRTUAL TABLE fts_messages USING fts5(...)`
- WAL mode: `PRAGMA journal_mode=WAL`
- `tokio::task::spawn_blocking` for all SQLite calls
- `rusqlite` with `"vtab"` feature flag
- `~/.talon/profiles/<name>/talon.db`

**Retrieval hierarchy:**
```
FTS5 (default, always on)
  └── Hybrid RRF fusion (optional, feature = "semantic-search")
        ├── fastembed-rs (local embeddings)
        └── sqlite-vec (vector storage)
```

---

## Cluster D — LLM Integration

**Cohesion:** High (0.13–0.22)
**Description:** All LLM provider bindings. Tight cluster — each provider doc is nearly self-contained.

| Doc | Title | Community | Cohesion |
|-----|-------|-----------|----------|
| 16 | LLM Provider Abstraction | 46 | 0.13 |
| 31 | Streaming & Real-Time Output | 60 | 0.14 |
| 41 | LLM Provider Abstraction Layer | 46 | 0.13 |
| 42 | OpenAI-Compatible Client | 98 | 0.22 |
| 43 | Anthropic Provider | 99 | 0.22 |
| 44 | Streaming SSE Parser | 48 | 0.13 |
| 47 | Ollama Local Provider | 64 | 0.14 |

**God Doc:** **Doc 41 — LLM Provider Abstraction Layer**
- Central hub connecting all provider impls
- Query `"LLM provider integration"` → 21 nodes, starts here
- Defines `LlmProvider` trait that Doc 42, 43, 47 all implement

**Key shared concepts:**
- `pub trait LlmProvider: Send + Sync`
- `async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError>`
- `async fn stream(&self, req: LlmRequest) -> Result<BoxStream<...>, LlmError>`
- `SseFrame` / `DeltaType` (Community 48)
- `#[async_trait]` macro on all impls

**Provider cohesion scores:**
- OpenAI-compat (C98: 0.22) and Anthropic (C99: 0.22) are the tightest docs in the entire project — most self-contained
- Ollama (C64: 0.14) is slightly looser due to model management and fallback chain

---

## Cluster E — Migration Path

**Cohesion:** Low-Medium (0.06–0.14) — deliberately broad
**Description:** The strangler fig plan. These docs explain HOW to migrate, not what to build. Lower cohesion is expected — they reference everything.

| Doc | Title | Community | Cohesion |
|-----|-------|-----------|----------|
| 21 | Migration Roadmap & Phases | 11 | 0.08 |
| 22 | Strangler Fig Pattern | 8 | 0.08 |
| 23 | Feature Flag Strategy | 8 | 0.08 |
| 24 | Async Migration (Node.js → Tokio) | 6 | 0.08 |
| 25 | Data Model Migration | 7 | 0.08 |
| 26 | Python-to-Rust Patterns | 0 | 0.06 |
| 27 | TypeScript-to-Rust Patterns | 2 | 0.06 |
| 28 | Test Strategy | 57 | 0.14 |

**God Doc:** **Doc 21 — Migration Roadmap & Phases**
- Graph god node with 12 edges (5th most connected node in entire graph)
- Community 11 contains all 7 migration phases + milestones table
- Every other migration doc is a deep-dive of one phase or one technique

**Why low cohesion is OK here:**
Migration docs reference concepts from ALL other clusters by design. Doc 22 (strangler fig) must reference the agent loop (Cluster A), tool system (Cluster B), and memory (Cluster C). Low community cohesion = high bridging value.

---

## Cluster F — Multi-Agent & Orchestration

**Cohesion:** Medium (0.11–0.17)
**Description:** Subagents, delegation, ACP protocol.

| Doc | Title | Community | Cohesion |
|-----|-------|-----------|----------|
| 19 | Subagent & Delegation Architecture | 62+26 | 0.11–0.14 |
| 51 | Channel Patterns | 27 | 0.11 |
| 53 | Subagent Delegation | 84 | 0.17 |

**God Doc:** **Doc 53 — Subagent Delegation**
- Community 84 (cohesion 0.17) is the tightest in this cluster
- "Parallel Batch Execution" + "Spawn Depth Limits" are the defining features

**Key shared concepts:**
- `tokio::task::JoinSet` for parallel subagent spawning
- `max_spawn_depth` config guard
- `DelegationEngine` with toolset filtering
- ACP protocol for cross-agent delegation (Community 88)

---

## Cluster G — Gateway & Delivery

**Cohesion:** Medium-High (0.13–0.22)
**Description:** All outbound channels. Self-contained per-platform.

| Doc | Title | Community | Cohesion |
|-----|-------|-----------|----------|
| 18 | Gateway Architecture | 73 | 0.15 |
| 34 | Send Message Tool | 32 | 0.12 |
| 45 | Telegram Gateway | 77 | 0.15 |
| 46 | Discord Gateway | 94 | 0.20 |
| 47 | Message Format & Normalization | 93 | 0.20 |

**God Doc:** **Doc 18 — Gateway & Multi-Channel Architecture**
- Community 73 is the structural hub: defines `Gateway` trait + routing
- Query `"gateway HTTP server"` → 13 nodes (tight cluster)
- All platform gateways (45, 46) implement the trait from Doc 18

---

## Cluster H — DevOps & Operations

**Cohesion:** Medium (0.11–0.18)

| Doc | Title | Community | Cohesion |
|-----|-------|-----------|----------|
| 60 | Build System | 51 | 0.13 |
| 61 | Docker & Container Deployment | 89 | 0.18 |
| 62 | Docker Build | 40 | 0.12 |
| 63 | CI/CD Pipeline | 30+79 | 0.11–0.15 |
| 64 | Config System | 70+72 | 0.14–0.15 |
| 65 | Release & Distribution | 29 | 0.11 |

**God Doc:** **Doc 63 — CI/CD Pipeline**
- Bridges test strategy (Doc 28), build system (Doc 60), and release (Doc 65)
- Contains both GitHub Actions definition AND local pre-commit hooks

---

## Cross-Cluster Bridges

These docs appear in **multiple clusters** — they are the architectural connective tissue:

| Doc | Title | Bridges |
|-----|-------|---------|
| 13 | Agent Loop | A (Core Runtime) ↔ B (Tools) ↔ D (LLM) |
| 14 | Tool System Architecture | A ↔ B ↔ C (via ToolContext holding MemoryStore ref) |
| 17 | Approval Membrane | B (Tools) ↔ G (Gateway) — all tool calls and all inbound messages pass through |
| 55 | SQLite + FTS5 | C (Memory) ↔ H (DevOps via Docker volume mounts) |
| 40 | Profile Isolation | C (Memory) ↔ H (Config) |
| 28 | Test Strategy | E (Migration) ↔ H (DevOps) |

---

## Cluster Cohesion Summary

```
Cluster A (Core Runtime)       ████████████  High     — build this first
Cluster B (Tool Ecosystem)     ███████████   High     — build second
Cluster C (Memory)             ██████████    High     — build third
Cluster D (LLM Integration)    ████████████  High     — can parallel with B
Cluster E (Migration Path)     ██████        Medium   — reference docs, low cohesion by design
Cluster F (Multi-Agent)        ████████      Medium   — build after A+B stable
Cluster G (Gateway)            ████████████  High     — mostly self-contained
Cluster H (DevOps)             ████████      Medium   — setup early, iterate throughout
```

---

*Generated from graphify community analysis — 102 communities, 1,667 nodes, 1,566 edges*
