# Capability Matrix — Keep / Edit / Drop

> **Last corrected:** dogfood pass 4
>
> **Status:** ✅ Complete
> **Category:** Analysis

---

## Legend

| Symbol | Meaning |
|--------|---------|
| ✅ KEEP | Port directly |
| ✏️ EDIT | Keep concept, rewrite implementation |
| ❌ DROP | Legacy, redundant, or wrong-fit for Rust |
| 🆕 NEW | Not in source — add because Rust enables it |

---

## 1. Agent Core

| Feature | Decision | Rationale |
|---------|----------|-----------|
| Autonomous agent loop | ✅ KEEP | Core product |
| JSON schema tool calling | ✏️ EDIT | Replace hand-written JSON with `schemars` derive (agent loop is sync; no asyncio) |
| Streaming LLM output | ✅ KEEP | Map to `futures::Stream` |
| Multi-step planning | ✅ KEEP | Agent loop configuration |
| Goal decomposition (ISA/ISC) | ✏️ EDIT | Explicit [state machine](../02_Architecture/14_State_Machine_And_Lifecycle.md), drop PAI's opaque Algorithm |
| Model routing | ✅ KEEP | `ModelPolicy` struct |
| AWS Bedrock transport | ✅ KEEP | Real provider; map to reqwest + SigV4 |
| Codex responses API | ✅ KEEP | `codex_responses` api_mode in run_agent.py |
| `--yolo` / autopilot | ✅ KEEP | `[ApprovalLevel](../02_Architecture/17a_Approval_Membrane.md)::Safe` flag |
| Max iteration guard | ✅ KEEP | `limits.rs` |
| Token budget tracking | ✏️ EDIT | tiktoken-rs |
| `[delegate_task](../04_Core_Features/37_Subagent_Delegation.md)` subagents | ✏️ EDIT | Tokio tasks + channels, no subprocess overhead |

---

## 2. Memory & Context

| Feature | Decision | Rationale |
|---------|----------|-----------|
| [SQLite + FTS5](../07_Memory_System/55_SQLite_FTS5_In_Rust.md) persistence | ✅ KEEP | rusqlite, same schema |
| MEMORY.md / USER.md / SOUL.md | ✅ KEEP | Plain markdown |
| AGENTS.md project context | ✅ KEEP | Scanned on session start |
| SKILL.md procedural memory | ✅ KEEP | Format-compatible with existing ecosystems |
| Three-tier memory | ✏️ EDIT | SQLite tagged categories, not separate dirs |
| Typed knowledge graph | ✏️ EDIT | SQLite entity table with JSON properties |
| Honcho [user modeling](../07_Memory_System/58a_User_Modeling.md) | ❌ DROP | External Python dep; USER.md covers the use case |
| [Embedding retrieval](../07_Memory_System/59_Embedding_Retrieval.md) | 🆕 NEW | Optional via `fastembed-rs`; FTS5 is default |
| History summarization | ✅ KEEP | Trigger at 80% of model token limit |

---

## 3. Tool System

| Feature | Decision | Rationale |
|---------|----------|-----------|
| terminal / shell tool | ✅ KEEP | `tokio::process::Command` |
| read_file / write_file / patch | ✅ KEEP | tokio::fs |
| search_files (ripgrep) | ✅ KEEP | spawn `rg` or `grep` crate |
| web_search | ✅ KEEP | reqwest + configurable backend |
| web_extract | ✅ KEEP | HTML→markdown via `scraper` + `comrak` |
| browser CDP tools | ✅ KEEP | `[chromiumoxide](../04_Core_Features/32_Browser_Tool.md)` |
| image_gen (FAL.ai) | ✅ KEEP | reqwest POST |
| TTS tools | ✅ KEEP | OpenAI TTS / ElevenLabs via reqwest |
| Docker sandbox backend | ✅ KEEP | `bollard` crate |
| SSH execution backend | ✅ KEEP | `russh` crate |
| MCP tool integration | ✅ KEEP | `rmcp` crate |
| Vercel sandbox | ❌ DROP | Cold starts incompatible with agent loops |
| [WASM plugin](../02_Architecture/17_Plugin_And_Skill_Architecture.md) tools | 🆕 NEW | `wasmtime` — plugins in any language |

---

## 4. Multi-Channel Gateway

| Feature | Decision | Rationale |
|---------|----------|-----------|
| Telegram | ✅ KEEP | `[teloxide](../05_API_Bindings/45_Telegram_Integration.md)` |
| Discord | ✅ KEEP | `serenity` |
| Slack | ✅ KEEP | HTTP Bolt via reqwest |
| WhatsApp | ✏️ EDIT | HTTP bridge to Baileys. v2. |
| Signal | ✏️ EDIT | `signal-cli` subprocess. v2. |
| iMessage / BlueBubbles | ❌ DROP | macOS-only, not portable |
| Matrix | ✏️ EDIT | `matrix-sdk` exists in Rust. v2. |
| DingTalk / WeCom / Weixin / Feishu / QQBot | ✏️ EDIT | CJK enterprise; v2. |
| Mattermost | ✏️ EDIT | Self-hosted; v2. |
| SMS | ✏️ EDIT | Twilio bridge; v2. |
| Yuanbao / Webhook / API server | ✏️ EDIT | Generic HTTP adapters |
| IRC | ❌ DROP | 2026. No. |
| Email (IMAP/SMTP) | ✅ KEEP | `lettre` + `imap` |
| Home Assistant | ✅ KEEP | HTTP API |
| HTTP gateway bridge | 🆕 NEW | External adapters forward via HTTP |

---

## 5. Scheduling

| Feature | Decision | Rationale |
|---------|----------|-----------|
| [Cron scheduler](../04_Core_Features/33_Cron_Scheduler.md) | ✅ KEEP | `tokio-cron-scheduler` |
| Human interval syntax | ✅ KEEP | `humantime` crate |
| Job persistence | ✅ KEEP | SQLite `cron_jobs` table |
| Rate-limit auto-resume | ✅ KEEP | Exponential backoff on 429 |
| Context chaining | ✅ KEEP | Store job output in SQLite |

---

## 6. Skill System

| Feature | Decision | Rationale |
|---------|----------|-----------|
| SKILL.md format | ✅ KEEP | Format-compatible with ClawHub |
| skills_list / skill_view | ✅ KEEP | LLM-accessible tools |
| skill_manage (create/patch/delete) | ✅ KEEP | Filesystem ops with validation |
| Skill hot-reload | ✅ KEEP | `notify` crate (inotify/FSEvents) |
| Auto-skill creation nudge | ✅ KEEP | Complexity score heuristic |
| Pinned skills | ✅ KEEP | `pinned: true` frontmatter |

---

## 7. Multi-Agent

| Feature | Decision | Rationale |
|---------|----------|-----------|
| Subagent delegation | ✏️ EDIT | Tokio tasks, not subprocess spawn |
| [ACP protocol](../05_API_Bindings/48_ACP_Protocol_Integration.md) | ✏️ EDIT | `axum` HTTP server |
| tmux parallel workers | ❌ DROP | GIL workaround — obsolete with Tokio |
| Staged pipeline | ✏️ EDIT | Configurable, not hardcoded |
| Orchestrator/worker roles | ✅ KEEP | `AgentRole` enum |
| Kanban multi-agent board | ✏️ EDIT | `plugins/kanban/` dispatcher; map to Tokio channels |
| `computer_use` tool | ✅ KEEP | Desktop/GUI automation; map to `enigo` / `chromiumoxide` |
| Observability plugin | ✅ KEEP | `plugins/observability/`; map to `tracing` + OpenTelemetry |

---

## 8. TUI / UX

| Feature | Decision | Rationale |
|---------|----------|-----------| 
| Multiline input | ✅ KEEP | `tui-textarea` |
| Streaming output | ✅ KEEP | [ratatui](../04_Core_Features/36_TUI_Implementation.md) paragraph |
| Slash-command autocomplete | ✏️ EDIT | Custom ratatui widget (Hermes TUI is TypeScript/React Ink, not Python/Rich) |
| HUD statusline | ✏️ EDIT | ratatui status bar (source: `ui-tui/` — Ink/React) |
| A2UI Live Canvas | ❌ DROP | Proprietary, out of scope v1 |
| Companion apps | ❌ DROP | Separate products |

---

## 9. Self-Evolution

| Feature | Decision | Rationale |
|---------|----------|-----------|
| DSPy + GEPA evolution | ✏️ EDIT | Python sidecar, HTTP API |
| Execution trace storage | ✅ KEEP | SQLite — sidecar reads |
| PR-based output | ✅ KEEP | Sidecar opens PRs |
| Trajectory generation | ✅ KEEP | SQLite → JSONL export |

---

## Summary

| Decision | Count |
|----------|-------|
| ✅ KEEP | 68 |
| ✏️ EDIT | 31 |
| ❌ DROP | 12 |
| 🆕 NEW | 5 |
| **Total** | **116** |
---

## Related Documents

### Depends On
- [OpenClaw Feature Audit](02_OpenClaw_Feature_Audit.md)
- [Hermes Agent Feature Audit](03_Hermes_Agent_Feature_Audit.md)

### See Also
- [Strategic Recommendations](10_Strategic_Recommendations.md)
- [Migration Roadmap](../03_Migration_Strategy/21_Migration_Roadmap.md)
- [Risk Register](../03_Migration_Strategy/28_Risk_Register.md)

