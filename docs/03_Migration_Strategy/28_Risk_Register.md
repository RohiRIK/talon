# Risk Register

> **Status:** ✅ Complete
> **Category:** Migration Strategy

---

## 1. Risk Scoring

Each risk is scored on two axes:
- **Likelihood**: 1 (rare) → 5 (near-certain)
- **Impact**: 1 (minor) → 5 (project-blocking)
- **Score** = Likelihood × Impact

---

## 2. Technical Risks

### R-T1: Playwright / Browser Tool Ecosystem Gap
| Field | Value |
|-------|-------|
| Likelihood | 4 |
| Impact | 3 |
| Score | 12 (High) |
| Description | `[chromiumoxide](../04_Core_Features/32_Browser_Tool.md)` (Rust) has lower community support than Playwright (TS) or `playwright-python`. Some advanced browser APIs may be missing or broken. |
| Mitigation | Use `chromiumoxide` for core navigation/snapshot; fall back to calling a headless Chromium process via CDP for edge cases. Keep browser tool interface abstract so the backend can be swapped. |
| Owner | Phase 6 |

---

### R-T2: ONNX Runtime (fastembed-rs) Build Complexity
| Field | Value |
|-------|-------|
| Likelihood | 3 |
| Impact | 2 |
| Score | 6 (Medium) |
| Description | `ort` (ONNX Runtime bindings) adds significant compile-time complexity, CI build times, and [cross-compilation](../08_DevOps/65_Release_And_Distribution.md) difficulty. Especially painful for ARM64 targets. |
| Mitigation | Feature-flag embeddings. Don't build `ort` unless explicitly requested. Default Docker image excludes it. |
| Owner | Phase 6 |

---

### R-T3: SQLite Concurrency Under Load
| Field | Value |
|-------|-------|
| Likelihood | 2 |
| Impact | 4 |
| Score | 8 (Medium-High) |
| Description | SQLite with WAL mode handles concurrent reads well but serializes writes. Under heavy concurrent subagent workloads (many cron + delegate tasks), write contention could become a bottleneck. |
| Mitigation | Use `[rusqlite](../07_Memory_System/55_SQLite_FTS5_In_Rust.md)` connection pool with write serialization via dedicated Tokio task channel. If throughput becomes an issue, migrate hot tables to a Postgres backend behind the same trait. |
| Owner | Phase 1 |

---

### R-T4: tokio-cron-scheduler API Instability
| Field | Value |
|-------|-------|
| Likelihood | 2 |
| Impact | 3 |
| Score | 6 (Medium) |
| Description | `tokio-cron-scheduler` is a younger crate. APIs may change between minor versions, and missed-run semantics are implementation-specific. |
| Mitigation | Thin wrapper trait over the scheduler. If crate becomes problematic, swap to cron-expression parsing + manual Tokio sleep loops (100 lines of code, fully owned). |
| Owner | Phase 5 |

---

### R-T5: MCP Protocol Version Drift
| Field | Value |
|-------|-------|
| Likelihood | 3 |
| Impact | 2 |
| Score | 6 (Medium) |
| Description | MCP ([Model Context Protocol](../05_API_Bindings/47_MCP_Protocol_Integration.md)) is actively evolving. The `rmcp` crate may lag behind spec changes. |
| Mitigation | Version-pin `rmcp`. Wrap in an internal `McpClient` trait. Absorb breakage in one place. |
| Owner | Phase 6 |

---

## 3. Ecosystem Risks

### R-E1: Anthropic API Breaking Changes
| Field | Value |
|-------|-------|
| Likelihood | 2 |
| Impact | 4 |
| Score | 8 (Medium-High) |
| Description | Anthropic may deprecate or change the `tool_use` format, streaming event types, or model identifiers without sufficient notice. |
| Mitigation | `[LlmProvider](../05_API_Bindings/41_LLM_Provider_Abstraction.md)` trait isolates Anthropic behind a clean interface. Version-pin the API format. Monitor Anthropic changelog. |
| Owner | Ongoing |

---

### R-E2: OpenRouter Rate Limits / Reliability
| Field | Value |
|-------|-------|
| Likelihood | 3 |
| Impact | 2 |
| Score | 6 (Medium) |
| Description | OpenRouter is a third-party proxy. Outages or policy changes could affect Talon users who rely on it as their primary provider. |
| Mitigation | Retry with exponential backoff. Provider fallback chain in config. Document that OpenRouter is best-effort. |
| Owner | Phase 1 |

---

### R-E3: Telegram Bot API Changes
| Field | Value |
|-------|-------|
| Likelihood | 2 |
| Impact | 3 |
| Score | 6 (Medium) |
| Description | Telegram periodically deprecates bot API methods. `[teloxide](../05_API_Bindings/45_Telegram_Integration.md)` tracks the API but may lag. |
| Mitigation | Keep Telegram gateway implementation thin. Version-pin `teloxide`. Test bot connectivity in CI with a smoke-test bot. |
| Owner | Phase 4 |

---

## 4. Migration Risks

### R-M1: Feature Parity Regression
| Field | Value |
|-------|-------|
| Likelihood | 3 |
| Impact | 4 |
| Score | 12 (High) |
| Description | Talon may silently miss edge-case behavior from OpenClaw/Hermes (e.g., specific tool error formats the LLM has learned to handle, or subtle session_search ranking differences). |
| Mitigation | Port the existing test suites. Add golden-output tests for session_search. Run both systems on the same task and compare outputs during transition. |
| Owner | Phase 3 |

---

### R-M2: SQLite Schema Migration Bugs
| Field | Value |
|-------|-------|
| Likelihood | 2 |
| Impact | 4 |
| Score | 8 (Medium-High) |
| Description | If a user migrates an existing Hermes/OpenClaw SQLite DB to Talon and the migration script has bugs, historical sessions could be corrupted or lost. |
| Mitigation | Migration script is read-only on the source DB (never modifies). Creates a new DB. Verify row counts before and after. Require `--confirm` flag. |
| Owner | Phase 2 |

---

### R-M3: Skill Format Incompatibility
| Field | Value |
|-------|-------|
| Likelihood | 2 |
| Impact | 2 |
| Score | 4 (Low) |
| Description | SKILL.md frontmatter format differences between Hermes and Talon could cause skill loading failures. |
| Mitigation | Talon's skill loader is tolerant — missing frontmatter fields get defaults. A one-time migration script normalizes existing skills. |
| Owner | Phase 2 |

---

## 5. Operational Risks

### R-O1: Container Security Escape
| Field | Value |
|-------|-------|
| Likelihood | 1 |
| Impact | 5 |
| Score | 5 (Medium) |
| Description | Talon executes arbitrary shell commands. A crafted tool call could attempt container escape if sandbox is misconfigured. |
| Mitigation | [seccomp profile](../08_DevOps/61_Docker_And_Container_Deployment.md) blocks dangerous syscalls. `cap_drop: ALL`. Read-only root filesystem. Non-root user in container. Defense in depth — see `20_Security_Model.md`. |
| Owner | Phase 1 |

---

### R-O2: API Key Leakage via Tool Output
| Field | Value |
|-------|-------|
| Likelihood | 2 |
| Impact | 5 |
| Score | 10 (High) |
| Description | A tool could inadvertently output an API key (e.g., `cat [config.toml](../02_Architecture/18a_Config_System.md)` or `env`). This output is stored in SQLite and potentially delivered to Telegram. |
| Mitigation | Secret scanner runs on all tool output before storage/delivery. Patterns: `sk-...`, `Bearer ...`, common key formats. Matches are redacted to `[REDACTED]`. Users are notified when redaction occurs. |
| Owner | Phase 1 |

---

## 6. Risk Summary Matrix

| ID | Risk | Score | Priority |
|----|------|-------|----------|
| R-T1 | Browser tool ecosystem gap | 12 | High |
| R-M1 | Feature parity regression | 12 | High |
| R-O2 | API key leakage | 10 | High |
| R-T3 | SQLite write contention | 8 | Medium |
| R-E1 | Anthropic API changes | 8 | Medium |
| R-M2 | DB migration bugs | 8 | Medium |
| R-T4 | [Cron scheduler](../04_Core_Features/33_Cron_Scheduler.md) instability | 6 | Medium |
| R-T2 | ONNX build complexity | 6 | Medium |
| R-E2 | OpenRouter reliability | 6 | Medium |
| R-E3 | Telegram API changes | 6 | Medium |
| R-T5 | MCP protocol drift | 6 | Medium |
| R-O1 | Container escape | 5 | Medium |
| R-M3 | Skill format incompatibility | 4 | Low |
---

## Related Documents

### Depends On
- [Migration Roadmap](21_Migration_Roadmap.md)

### See Also
- [Rust Migration Tradeoffs](../01_Analysis/09_Rust_Migration_Tradeoffs.md)
- [Security Model](../02_Architecture/20_Security_Model.md)

