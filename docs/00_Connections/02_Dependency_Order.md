# Dependency Order & Implementation DAG

> **Purpose:** If you're building Talon from scratch, this is the order to implement things.
> Based on graphify community analysis + logical data-flow dependencies between concepts.
> "X → Y" means "X must exist before Y can be implemented."

---

## The Core Rule

A doc has a dependency on another when:
1. It uses a type, trait, or struct defined in that doc
2. It calls a function/method from that doc's module
3. Its tests require that doc's system to be running

---

## Full Dependency DAG (ASCII)

```
═══════════════════════════════════════════════════════════════
PHASE 0 — FOUNDATION (Week 1)
═══════════════════════════════════════════════════════════════

 Doc 60 (Build System / Cargo Workspace)
   │   "Set up crates/ layout, root Cargo.toml, feature flags"
   ▼
 Doc 12 (Cargo Workspace Design)
   │   "Agree on crate names: talon-core, talon-llm, etc."
   ▼
 Doc 11 (System Architecture Overview)
   │   "Understand the full data flow before writing any code"
   │
   ├──► Doc 50 (Tokio Runtime Configuration)
   │      "Configure #[tokio::main] and worker threads"
   │
   └──► Doc 54 (Error Handling Strategy)
          "Define AgentError, LlmError, ToolError, MemoryError"
          "before any other crate compiles against them"

═══════════════════════════════════════════════════════════════
PHASE 1 — CORE AGENT LOOP (Weeks 2–3)
═══════════════════════════════════════════════════════════════

 Doc 50 + Doc 54
   │
   ▼
 Doc 16 / Doc 41 (LLM Provider Abstraction)
   │   "Define LlmProvider trait + LlmRequest/LlmResponse types"
   │   "Implement OpenAI-compat stub first (Doc 42)"
   │
   ├──► Doc 42 (OpenAI-Compatible Client)   ← implement first
   ├──► Doc 43 (Anthropic Provider)          ← implement second
   ├──► Doc 44 (Streaming SSE Parser)        ← needed by both
   └──► Doc 47 (Ollama Local Provider)       ← implement third

 Doc 54 (Errors)
   │
   ▼
 Doc 14 (Tool System Architecture)
   │   "Define Tool trait, ToolContext, ToolResult, ApprovalLevel"
   │   "This is the contract every tool impl signs"
   │
   ▼
 Doc 17 (Approval Membrane)
   │   "Wire ApprovalMembrane into ToolContext"
   │   "Blocks all tool execution until safety check passes"

 Doc 41 (LLM Provider) + Doc 14 (Tool Trait) + Doc 17 (Approval)
   │
   ▼
 Doc 13 (Agent Loop Implementation)       ◄── CRITICAL PATH
   │   "The convergence point — assembles everything"
   │   "Cannot be built until 41+14+17 all compile"
   │
   └──► Doc 20 (State Machine & Agent Lifecycle)
          "Wrap the loop in proper states: Idle/Running/Waiting/Done"

═══════════════════════════════════════════════════════════════
PHASE 2 — MEMORY (Weeks 3–4)
═══════════════════════════════════════════════════════════════

 Doc 12 (Workspace)
   │
   ▼
 Doc 55 (SQLite + FTS5 in Rust)
   │   "Create talon.db, sessions/messages tables, FTS5 vtab"
   │   "Establish spawn_blocking pattern for all DB calls"
   │
   ├──► Doc 57 (Session Search)
   │      "Wire FTS5 queries into session_search tool"
   │
   └──► Doc 58 (FTS5 Deep Dive)
          "Advanced query syntax, BM25 tuning"

 Doc 55
   │
   ▼
 Doc 15 (Context & Memory Architecture)
   │   "Memory tiers: MEMORY.md → session → messages"
   │   "Context assembly pipeline for each turn"
   │
   └──► Doc 35 (Memory System Overview)
          "Higher-level API over Doc 55's raw SQLite"

 Doc 55 (optional feature)
   │
   └──► Doc 56 (Embedding Retrieval)
          "fastembed-rs + sqlite-vec, hybrid RRF fusion"
          "Gated behind feature = 'semantic-search'"

 Doc 15 + Doc 35
   │
   └──► Doc 66 (User Model — USER.md)
          "Inject USER.md into every context assembly"

═══════════════════════════════════════════════════════════════
PHASE 3 — TOOLS TIER 1 (Weeks 4–5)
═══════════════════════════════════════════════════════════════

 Doc 14 (Tool Trait) + Doc 17 (Approval)
   │
   ├──► Doc 59 (File System Tools)    ← implement first (simplest)
   │      "ReadFile, WriteFile, Patch, SearchFiles"
   │      "All Safe approval level"
   │
   ├──► Doc 29 (Terminal Tool)        ← implement second
   │      "Dangerous approval level"
   │      "Docker sandbox backend"
   │
   └──► Doc 30 (Tool Execution Engine)
          "Parallel tool execution via JoinSet"
          "Wraps all tool impls with timeout + error handling"

 Doc 14 + Doc 30
   │
   └──► Doc 52 (Async Tool Execution)
          "spawn_blocking bridge for sync tool impls"
          "Timeout wrapper around every tool call"

═══════════════════════════════════════════════════════════════
PHASE 4 — GATEWAY (Weeks 5–6)
═══════════════════════════════════════════════════════════════

 Doc 12 (Workspace)
   │
   ▼
 Doc 18 (Gateway Architecture)
   │   "Define Gateway trait + AgentInput/AgentOutput types"
   │
   ├──► Doc 47 (Message Format & Normalization)
   │      "normalize_for_platform(), split_message()"
   │
   ├──► Doc 45 (Telegram Gateway)     ← implement first
   │      "teloxide, long polling, webhook mode"
   │
   ├──► Doc 46 (Discord Gateway)      ← implement second
   │
   └──► Doc 34 (Send Message Tool)
          "LLM-facing tool that calls Gateway::deliver()"

═══════════════════════════════════════════════════════════════
PHASE 5 — TOOLS TIER 2 (Weeks 6–7)
═══════════════════════════════════════════════════════════════

 Doc 14 (Tool Trait)
   │
   ├──► Doc 32 (Browser Tool)
   │      "chromiumoxide, BrowserPool, accessibility snapshot"
   │
   ├──► Doc 33 (Web Search & Extract)
   │      "Brave Search / SearXNG backends"
   │
   └──► Doc 36 (MCP Client Tool)
          "McpToolAdapter wrapping Tool trait"

═══════════════════════════════════════════════════════════════
PHASE 6 — PLUGIN + SCHEDULING (Weeks 7–8)
═══════════════════════════════════════════════════════════════

 Doc 14 (Tool Trait) + Doc 13 (Agent Loop)
   │
   ├──► Doc 17b (Plugin Architecture — WASM)
   │      "extism runtime, WasmTool implements Tool"
   │
   ├──► Doc 38 (Skill Store)
   │      "SKILL.md parsing, hot-reload via notify"
   │
   └──► Doc 37 (Cron & Scheduling)
          "CronStore (SQLite), CronJob type, scheduler loop"
          "Requires: Doc 20 (State Machine) + Doc 55 (SQLite)"

═══════════════════════════════════════════════════════════════
PHASE 7 — ADVANCED FEATURES (Weeks 8+)
═══════════════════════════════════════════════════════════════

 Doc 13 (Agent Loop) + Doc 50 (Tokio)
   │
   ├──► Doc 19 / Doc 53 (Subagent Delegation)
   │      "JoinSet-based parallel subagent spawning"
   │      "max_spawn_depth guard"
   │
   └──► Doc 39 (Self-Evolution Loop)
          "GEPA + DSPy pipeline against Talon"
          "Requires stable agent loop (Phase 1)"

 Doc 40 (Profile Isolation) → Doc 64 (Config System)
   "Profiles must exist before config loading by profile name"

═══════════════════════════════════════════════════════════════
ANALYSIS + DEVOPS (Parallel Track — any phase)
═══════════════════════════════════════════════════════════════

 Doc 01–10 (Analysis)     ← read before writing any code
 Doc 21–27 (Migration)    ← reference during each phase
 Doc 28 (Test Strategy)   ← wire tests as you go
 Doc 61–62 (Docker)       ← set up Docker in Phase 0
 Doc 63 (CI/CD)           ← set up GitHub Actions in Phase 0
 Doc 65 (Release)         ← prep in Phase 6, execute in Phase 7
```

---

## Condensed Linear Reading Order

For someone approaching the docs cold, read in this order:

### Orientation (1–2 hours)
1. `Doc 01` — What is Talon, where it comes from
2. `Doc 11` — System architecture bird's-eye view
3. `Doc 10` — Strategic decisions and guiding principles
4. `Doc 21` — Migration roadmap phases

### Core Design (2–3 hours)
5. `Doc 12` — Cargo workspace layout
6. `Doc 54` — Error handling (read early — shapes everything)
7. `Doc 14` — Tool trait contract
8. `Doc 41` — LLM provider abstraction

### Implementation Depth (per feature being built)
- **Building the loop?** → Docs 13, 20, 50, 51
- **Building a tool?** → Docs 14, 17, 30, 52, 54
- **Building memory?** → Docs 15, 35, 55, 56, 57
- **Building a gateway?** → Docs 18, 45, 47
- **Setting up LLM?** → Docs 41, 42, 43, 44
- **CI/CD setup?** → Docs 28, 60, 62, 63

---

## Dependency Matrix (simplified)

`●` = hard dependency (must build first)
`○` = soft dependency (helpful but can stub)
`—` = no dependency

| Building → | Needs Doc 12 | Needs Doc 14 | Needs Doc 41 | Needs Doc 54 | Needs Doc 55 | Needs Doc 13 |
|---|---|---|---|---|---|---|
| Doc 13 (Agent Loop) | ● | ● | ● | ● | ○ | — |
| Doc 14 (Tool Trait) | ● | — | — | ● | — | — |
| Doc 30 (Tool Engine) | ● | ● | — | ● | — | ○ |
| Doc 42 (OpenAI) | ● | — | ● | ● | — | — |
| Doc 45 (Telegram) | ● | — | — | ● | — | ○ |
| Doc 55 (SQLite) | ● | — | — | ● | — | — |
| Doc 56 (Embeddings) | ● | — | — | ● | ● | — |
| Doc 37 (Cron) | ● | ● | — | ● | ● | ● |
| Doc 39 (Evolution) | ● | ● | ● | ● | ● | ● |
| Doc 53 (Subagents) | ● | ● | ● | ● | — | ● |

---

## What You Can Build in Parallel

These have no shared dependencies and can be built simultaneously:

**Parallel stream 1:** LLM providers (Docs 42, 43, 47)
- All implement `LlmProvider` but don't depend on each other

**Parallel stream 2:** Gateway platforms (Docs 45, 46)
- All implement `Gateway` but don't depend on each other

**Parallel stream 3:** Safe tools (Docs 59, 33, 32)
- All implement `Tool` but don't depend on each other

**Parallel stream 4:** Memory subsystem (Docs 55 → 56, 57, 58)
- Linear within the stream but independent from LLM/Gateway

---

## Critical Path (fastest to a working agent)

```
Doc 60 → Doc 12 → Doc 54 → Doc 14 → Doc 41 → Doc 42
                                              ↓
Doc 55 → Doc 15 ──────────────────────────► Doc 13
                                              ↓
Doc 59 (file tools — simplest) ────────────► WORKING AGENT
                                              (can read files, call LLM, loop)
```

**Estimated time to working MVP:** 2–3 weeks of focused Rust work.
The critical path is 8 docs. Everything else is features on top.

---

*Based on graphify community analysis — dependency edges inferred from concept co-location in communities 46, 55, 58, 74, 78, 96, 97*
