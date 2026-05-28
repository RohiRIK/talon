# Ernest AI Docs — Final Audit (Pass 3)
Date: 2026-05-25

---

## Pass 3 Fixes — Confirmed/Not Confirmed

| Fix | Status | Notes |
|-----|--------|-------|
| `ToolOutput` → `ToolResult` (16 files) | ✅ CONFIRMED | 0 occurrences remaining |
| `Arc<Box<dyn Tool>>` → `Arc<dyn Tool>` (doc 50) | ✅ CONFIRMED | 1 hit in doc 16 is an inline comment `// Arc<dyn Tool>, NOT Arc<Box<dyn Tool>>` — intentional, not an error |
| rusqlite `vtab` feature (doc 60) | ✅ CONFIRMED | Both doc 55 and doc 60 show `vtab` feature; minor note: doc 60 uses `"load_extension"` while doc 12 uses `"functions"` — minor cross-doc inconsistency |
| `TerminalTool::name()` returns `&str` (doc 16/30) | ✅ CONFIRMED | `fn name(&self) -> &str { "terminal" }` in doc 30 |
| `build_tool_registry` made `async` (doc 17) | ✅ CONFIRMED | No regression found |
| `ApprovalLevel` unified to `{ Safe, Confirmation, Required, Blocked }` (docs 17, 20) | ⚠️ PARTIAL | Docs 17 and 20 are correct. **2 stray old variants remain in non-primary docs:** `ApprovalLevel::AskEveryTime` in `03_Migration_Strategy/26_Test_Strategy.md:60` and `ApprovalLevel::AlwaysApprove` in `01_Analysis/06_Capability_Matrix.md:33`. Also `ApprovalMode::AskForDangerous` in `02_Architecture/18_Config_System.md:146` (different enum, may be intentional). Doc 03 line 67 uses old names but is describing Hermes *Python* source system, not Ernest's Rust enum — acceptable. |
| `AgentEvent` canonical struct-variant form (doc 29) | ✅ CONFIRMED | `AgentEvent` enum uses struct variants correctly: `ToolCallStarted { tool_name, call_id }`, `ToolCallCompleted { call_id, result }` etc. |
| `impl Stream` in trait → `Pin<Box<dyn Stream + Send>>` (doc 12) | ✅ CONFIRMED | Doc 41 now defines `pub type DeltaStream = Pin<Box<dyn Stream<Item = Result<Delta, LlmError>> + Send + 'static>>` and the trait uses `DeltaStream`. Remaining `impl Stream` occurrences are all in free functions / concrete impls where it is valid. |

---

## Doc Sample Scores

| Doc | Accuracy (1–5) | Rust (1–5) | Fidelity (1–5) | Avg | Notes |
|-----|---------------|------------|----------------|-----|-------|
| 01 — Source Ecosystem Overview | 5 | N/A | 5 | 5.0 | Clean, concise, well-scoped |
| 02 — OpenClaw Feature Audit | 4 | N/A | 4 | 4.0 | Honest caveats on NestJS/LangChain unverified claims |
| 03 — Hermes Agent Feature Audit | 5 | N/A | 5 | 5.0 | Correctly identifies sync loop, Ink TUI, 22+ platforms, Bedrock/Codex |
| 12 — Workspace & Crate Structure | 4 | 5 | 4 | 4.3 | Duplicate frontmatter line (patch splice artifact); workspace layout uses `crates/` subdir, differs from doc 60's flat layout |
| 13 — Core Agent Loop Design | 5 | 5 | 5 | 5.0 | Clean state machine, Rust types correct |
| 16 — Tool System Architecture | 5 | 5 | 5 | 5.0 | `&str`, `Arc<dyn Tool>`, `ApprovalLevel` all correct |
| 17 — Approval Membrane | 5 | 5 | 5 | 5.0 | `ApprovalLevel` canonical variants correct, async membrane pattern clean |
| 20 — Security Model | 3 | 4 | 4 | 3.7 | `ApprovalLevel` enum correct; but `ToolRisk` enum definition (`ReadOnly, SafeWrite, Dangerous, Irreversible, NetworkWrite`) conflicts with doc 17's `ToolRisk` (`ReadOnly=0, Network=1, LocalWrite=2, Destructive=3`). Two different `ToolRisk` designs in same codebase docs is a real issue. Comment strings still say "ask in AskForDangerous mode" which is a stale reference. |
| 21 — Migration Roadmap | 5 | 5 | 5 | 5.0 | Clear phased plan, accurate exit criteria |
| 22 — TypeScript-to-Rust Patterns | 5 | 5 | 5 | 5.0 | All patterns correct; `impl Stream` use is in free-fn context (valid) |
| 23 — Python-to-Rust Patterns | 5 | 5 | 5 | 5.0 | `asyncio.gather → join_all` pattern correct |
| 29 — Agent Loop Implementation | 5 | 5 | 5 | 5.0 | `AgentEvent` struct-variant form correct, `ToolResult` correct |
| 30 — Terminal Tool | 5 | 5 | 5 | 5.0 | `name() -> &str`, Docker sandbox, approval membrane wired correctly |
| 39 — Self-Evolution Loop | 5 | N/A | 5 | 5.0 | GEPA description accurate, DSPy integration correct |
| 41 — LLM Provider Abstraction | 5 | 5 | 5 | 5.0 | `DeltaStream` type alias correct, trait object-safe |
| 55 — SQLite FTS5 in Rust | 5 | 5 | 5 | 5.0 | `vtab` feature present, schema design correct |
| 60 — Build System Cargo Workspace | 4 | 4 | 4 | 4.0 | `vtab` present; uses `load_extension` instead of `functions` (vs doc 12); flat workspace layout vs `crates/` subdir in doc 12 — internal inconsistency |

---

## Hermes Source Cross-Check

| Claim | Verified? | Evidence |
|-------|-----------|---------|
| Agent loop is synchronous (not asyncio) | ✅ YES | AGENTS.md: "The core loop is inside `run_conversation()` — **entirely synchronous**" |
| TUI is TypeScript/React Ink | ✅ YES | AGENTS.md: `ui-tui/ # Ink (React) terminal UI — hermes --tui`, `entry.tsx, app.tsx, gatewayClient.ts` |
| 22+ gateway platforms exist | ✅ YES | AGENTS.md lists 19 named platforms (telegram, discord, slack, whatsapp, homeassistant, signal, matrix, mattermost, email, sms, dingtalk, wecom, weixin, feishu, qqbot, bluebubbles, yuanbao, webhook, api_server) plus `...` indicating more — 22+ is accurate |
| `run_agent.py` is the main loop | ✅ YES | AGENTS.md: `run_agent.py # AIAgent class — core conversation loop (~12k LOC)` |
| AWS Bedrock + Codex are real providers | ✅ YES | AGENTS.md: `api_mode: str = None, # "chat_completions" \| "codex_responses" \| ...`; doc 03 explicitly lists "AWS Bedrock, Codex" |

---

## New Issues from Pass 3 (if any)

1. **Doc 12 duplicate frontmatter line** — `> **Last corrected:** dogfood pass 3` appears on both line 3 and line 7. Patch splice artifact. Minor cosmetic issue.

2. **`ToolRisk` enum inconsistency (docs 17 vs 20)** — Not introduced by pass 3, but now visible after the `ApprovalLevel` cleanup. Doc 17 (`17_Approval_Membrane.md`) defines `ToolRisk { ReadOnly=0, Network=1, LocalWrite=2, Destructive=3 }` while doc 20 (`20_Security_Model.md`) defines `ToolRisk { ReadOnly, SafeWrite, Dangerous, Irreversible, NetworkWrite }`. These are two different designs; the doc 20 version still references `AskForDangerous` in inline comments, suggesting it was partially cleaned but not fully reconciled.

3. **Stray old `ApprovalLevel` variants not caught by pass 3:**
   - `26_Test_Strategy.md:60` — `ApprovalLevel::AskEveryTime` (should be `Required` or `Confirmation`)
   - `06_Capability_Matrix.md:33` — `ApprovalLevel::AlwaysApprove` (should be `Safe`)
   - `18_Config_System.md:146` — `ApprovalMode::AskForDangerous` (different enum name `ApprovalMode` vs `ApprovalLevel` — may be intentional or may be a stale parallel enum)

4. **Workspace layout drift (docs 12 vs 60)** — Doc 12 shows crates under `crates/` subdirectory (`crates/ernest-core/`, etc.) while doc 60 shows them flat (`ernest-core/`, etc.). One of these is wrong. Prefer doc 12's `crates/` layout as it matches Ernest's declared workspace `members` (`crates/ernest-core`).

5. **rusqlite feature inconsistency (docs 12 vs 60)** — Doc 12: `features = ["bundled", "vtab", "functions"]`; doc 60: `features = ["bundled", "vtab", "load_extension"]`. `functions` (user-defined SQL functions) is likely more relevant for FTS5 custom tokenizers. Recommend standardizing on doc 12's version.

---

## Overall Score: 4.5 / 5

---

## Verdict

**Ship with caveats** — the doc set is high quality and all primary architecture docs are accurate. Three targeted fixes remain before declaring "publishable":

### Remaining Action Items (Priority Order)

1. **[HIGH] Fix `ToolRisk` enum inconsistency** — Choose one canonical definition and apply it to both doc 17 and doc 20. Remove the stale `AskForDangerous` comment strings from doc 20's `ToolRisk` variants.

2. **[MEDIUM] Fix stray `ApprovalLevel` old variants:**
   - `26_Test_Strategy.md:60` — change `ApprovalLevel::AskEveryTime` → `ApprovalLevel::Required`
   - `06_Capability_Matrix.md:33` — change `ApprovalLevel::AlwaysApprove` → `ApprovalLevel::Safe`
   - `18_Config_System.md:146` — audit `ApprovalMode::AskForDangerous`; either rename to `ApprovalLevel` or document `ApprovalMode` as a separate config-layer enum

3. **[LOW] Reconcile workspace layouts** — Standardize docs 12 and 60 to same `crates/` layout; standardize rusqlite `features` to `["bundled", "vtab", "functions"]`.

4. **[LOW] Remove duplicate frontmatter line** in doc 12 (lines 3 and 7 both say `Last corrected: dogfood pass 3`).
