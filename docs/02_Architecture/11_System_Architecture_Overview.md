# Talon System Architecture Overview

> **Status:** ✅ Complete
> **Category:** Architecture

---

## 1. System Diagram

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           TALON RUNTIME                                │
│                                                                         │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐ │
│  │ CLI/TUI  │  │ Telegram │  │ Discord  │  │ HTTP API │  │  Cron    │ │
│  │(ratatui) │  │(teloxide)│  │(serenity)│  │ (axum)   │  │ Scheduler│ │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └────┬─────┘  └────┬─────┘ │
│       │              │              │              │              │       │
│       └──────────────┴──────────────┴──────────────┴──────────────┘      │
│                                    │                                     │
│                          ┌─────────▼──────────┐                         │
│                          │   Gateway Router   │                         │
│                          │ (talon-gateway)   │                         │
│                          └─────────┬──────────┘                         │
│                                    │                                     │
│                          ┌─────────▼──────────┐                         │
│                          │   Agent Session    │                         │
│                          │   Manager          │                         │
│                          │ (Arc<Mutex<>>)     │                         │
│                          └─────────┬──────────┘                         │
│                                    │                                     │
│                 ┌──────────────────▼──────────────────┐                 │
│                 │            Core Agent Loop           │                 │
│                 │           (talon-core)              │                 │
│                 │                                      │                 │
│                 │  Build Context ──→ LLM Call ──→ Parse│                 │
│                 │       ↑              │               │                 │
│                 │       │              ▼               │                 │
│                 │  Update Memory ← Execute Tools       │                 │
│                 └──────────────────────────────────────┘                 │
│                          │          │          │                         │
│               ┌──────────┘   ┌──────┘   ┌─────┘                        │
│               ▼              ▼           ▼                              │
│     ┌──────────────┐  ┌──────────────┐  ┌──────────────┐               │
│     │ Tool Registry│  │ Memory Store │  │ LLM Providers│               │
│     │(talon-tools)│  │(talon-mem.) │  │ (talon-llm) │               │
│     └──────────────┘  └──────┬───────┘  └──────────────┘               │
│                              │                                          │
│                    ┌─────────▼──────────┐                              │
│                    │   SQLite (WAL)     │                              │
│                    │  sessions/messages │                              │
│                    │  fts5/skills/cron  │                              │
│                    └────────────────────┘                              │
└─────────────────────────────────────────────────────────────────────────┘
                         │ (optional)
              ┌──────────▼──────────┐
              │  Self-Evolution     │
              │  Sidecar (Python)   │
              │  DSPy + GEPA        │
              └─────────────────────┘
```

---

## 2. Crate Responsibilities

| Crate | Role | Key Types |
|-------|------|-----------|
| `talon` | Binary entrypoint, wires everything | `main()`, `Config` load |
| `talon-core` | Agent loop, [approval membrane](17a_Approval_Membrane.md) | `Agent`, `AgentLoop`, `Context`, `Turn` |
| `talon-tools` | Built-in tool implementations | `ToolRegistry`, `Tool` trait, each tool |
| `talon-memory` | [SQLite + FTS5](../07_Memory_System/55_SQLite_FTS5_In_Rust.md), skills, user model | `MemoryStore`, `SessionStore`, `SkillStore` |
| `talon-llm` | [LLM provider abstraction](../05_API_Bindings/41_LLM_Provider_Abstraction.md) | `LlmProvider` trait, `Message`, `Delta` |
| `talon-gateway` | Channel adapters + HTTP gateway | `Gateway` trait, per-platform impls |
| `talon-plugins` | [WASM plugin](17_Plugin_And_Skill_Architecture.md) loader | `PluginHost`, `WasmTool` |

---

## 3. Core Data Flow

### Turn Lifecycle

```
1. Gateway receives user message
2. SessionManager maps it to a session (create if new)
3. AgentLoop.run(session_id, message):
   a. Load memory (MEMORY.md, skills summary, last N turns from SQLite)
   b. Assemble context window (system prompt + memory + history)
   c. POST to LlmProvider → stream Delta events
   d. Buffer tool_use blocks
   e. For each tool call:
      - Check ApprovalMembrane
      - Execute Tool
      - Stream result to gateway
   f. Append turn to SQLite
   g. Update MEMORY.md if auto-update triggered
4. Final assistant message → gateway.send(response)
```

---

## 4. Key Design Principles

1. **Thin core, thick plugins** — talon-core has no opinion on which tools exist
2. **No global state** — All state flows through `Arc<>` refs passed explicitly
3. **Approval membrane** — Every tool execution checks user permission level before running
4. **SQLite as source of truth** — All sessions, messages, cron jobs, skills persisted
5. **Stream everything** — LLM responses stream to gateway as they arrive; never buffer a full completion
6. **Error audiences** — Three representations: user-facing clean message, developer structured log, LLM-facing tool error
---

## Related Documents

### Depends On
- [Strategic Recommendations](../01_Analysis/10_Strategic_Recommendations.md)

### Used By
- [Cargo Workspace Design](12_Workspace_And_Crate_Structure.md)
- [Migration Roadmap](../03_Migration_Strategy/21_Migration_Roadmap.md)

### See Also
- [Core Agent Loop Design](13_Core_Agent_Loop_Design.md)
- [Security Model](20_Security_Model.md)

