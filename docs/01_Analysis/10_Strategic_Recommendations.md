# Strategic Recommendations & Guiding Principles

> **Status:** ✅ Complete
> **Category:** Analysis

---

## 1. The Bitter-Pill Principle

Borrowed from PAI's "bitter-pill engineering": **as models improve, the framework should shrink**. Every abstraction added today is debt that must be removed tomorrow when the model no longer needs it.

**Operational rule:** If you are writing code to compensate for a model's weakness, that code has an expiry date. Document it as such. Prefer data (prompts, skills, memory) over code (logic, parsers, validators) wherever possible.

---

## 2. Thin Core, Thick Periphery

```
Talon CORE (must be lean):
  - Agent loop
  - Tool dispatch
  - Context assembly
  - LLM streaming

Talon PERIPHERY (can be rich):
  - Individual tool implementations
  - Gateway adapters
  - WASM plugins
  - Self-evolution sidecar
```

Core changes should be rare and reviewed carefully. Periphery changes are cheap and frequent.

---

## 3. SQLite as the Single Source of Truth

Every piece of persistent state lives in SQLite:

| What | Where |
|------|-------|
| Sessions + messages | `messages` table |
| Full-text search index | `fts_messages` FTS5 |
| Scheduled jobs | `cron_jobs` table |
| Skill index | `skills` table |
| Tagged memory entries | `memory_entries` table |

**Never** scatter state across Redis, a separate vector DB, and flat files. One SQLite file = one `cp` for backup. One `sqlite3` CLI for inspection. Zero infra deps.

The exception: MEMORY.md / USER.md stay as markdown files because they are LLM-editable prose, not structured data.

---

## 4. The Approval Membrane is Non-Negotiable

Every agent action that touches the outside world passes through an explicit approval check. There is no "just do it" default — `--yolo` is an explicit user opt-in that must be set per-session.

```
READ   → always allowed
WRITE  → allowed by default, auditable
EXEC   → ask unless --yolo
PUSH   → always ask, even in --yolo mode (configurable)
```

The cost of a false positive (asking when you don't need to) is one user keystroke. The cost of a false negative (executing a destructive action silently) is potentially irreversible.

---

## 5. The Self-Evolution Boundary

Keep `[self-evolution](../04_Core_Features/39_Self_Evolution_Loop.md)` as a Python sidecar. Reasons:

1. DSPy and GEPA have no Rust equivalents worth building
2. Evolution is offline/periodic — not latency-sensitive
3. The boundary (HTTP API + SQLite trajectory export) is clean and stable
4. When Rust ML tooling matures, the sidecar can be replaced without touching Talon core

**Contract:** Talon writes execution traces to SQLite. Sidecar reads traces, runs GEPA, opens a PR with updated skills/prompts. No shared memory, no in-process coupling.

---

## 6. Feature Flag Discipline

Talon ships as one binary with compile-time feature flags:

| Flag | Default | Adds |
|------|---------|------|
| `voice` | off | whisper-rs, rodio |
| `vision` | off | [chromiumoxide](../04_Core_Features/32_Browser_Tool.md) |
| `embeddings` | off | [fastembed](../07_Memory_System/59_Embedding_Retrieval.md)-rs (~300MB model) |
| `evolution` | off | trajectory collection tables |
| `metrics` | off | Prometheus /metrics endpoint |

The default binary is lean. Power users opt in.

---

## 7. Platform SDK Priority

Build Tier-1 platforms natively. Use HTTP gateway bridges for everything else:

**Tier 1 (native Rust SDKs):**
- CLI (no SDK needed)
- Telegram (`[teloxide](../05_API_Bindings/45_Telegram_Integration.md)`)
- Discord (`serenity`)

**Tier 2 (HTTP gateway bridge — thin TypeScript adapter):**
- WhatsApp, Signal, Slack, Teams

**Tier 3 (defer to v2):**
- Matrix, WeChat, IRC

---

## 8. Observability from Day One

Don't bolt on logging after the fact. Every `async fn` in the hot path gets a `#[tracing::instrument]` span from day one. The agent loop emits structured events for every:
- LLM call (model, tokens, latency)
- Tool execution (name, args hash, latency, success/fail)
- Memory operation (type, size)
- Gateway delivery (platform, success/fail)

This is not optional overhead — it's what makes autonomous agents debuggable.

---

## 9. Test Philosophy

| Level | What | Tools |
|-------|------|-------|
| Unit | Pure functions, type conversions, schema validation | `#[test]` |
| Integration | SQLite ops, tool execution with mocked I/O | `tokio::test` |
| Contract | LLM provider response shapes | Recorded HTTP fixtures (`httpmock`) |
| E2E | Full agent turn with real LLM | `tests/e2e/` (CI-only) |

Target: **80% coverage on core crate**, 60% on tools, skip E2E in local dev.

---

## 10. Version 1 Scope — What NOT to Build Yet

| Feature | Rationale for deferral |
|---------|----------------------|
| Voice input (Whisper) | Complex, adds 300MB dep, v2 |
| Companion mobile app | Separate product |
| Multi-node agent cluster | Single-node is sufficient for v1 |
| Custom model fine-tuning | Self-evolution sidecar covers this |
| Web UI / dashboard | TUI + Telegram are sufficient |
| Matrix / WeChat gateway | Low demand, complex auth |
| GPU-accelerated embeddings | FTS5 is sufficient for v1 |

Ship a small, correct, fast v1. Expand from there.
---

## Related Documents

### Depends On
- [Capability Matrix](06_Capability_Matrix.md)
- [Rust Migration Tradeoffs](09_Rust_Migration_Tradeoffs.md)

### Used By
- [System Architecture Overview](../02_Architecture/11_System_Architecture_Overview.md)
- [Migration Roadmap](../03_Migration_Strategy/21_Migration_Roadmap.md)

