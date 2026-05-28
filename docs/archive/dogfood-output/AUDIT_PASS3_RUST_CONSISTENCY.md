# Rust Type Consistency Audit — Pass 3 (Final)

> Generated: 2026-05-25
> Auditor: Hermes subagent (automated)

---

## Pattern Search Results

| Pattern | Count | Files (if > 0) |
|---|---|---|
| `ToolOutput` | **0** | — ✅ |
| `Arc<Box<dyn Tool>>` | **0** (comment only) | `16_Tool_System_Architecture.md` line 69 — this is a corrective NOTE comment: `// Arc<dyn Tool>, NOT Arc<Box<dyn Tool>>` — not a live type, ✅ |
| `AlwaysAsk\|AskForDangerous\|AlwaysApprove\|ApproveOnce\|AskEveryTime\|AlwaysDeny` | **6** | ⚠️ **3 real violations** (see below) |
| `impl Stream` in trait method signatures | **1** | ⚠️ `22_TypeScript_To_Rust_Patterns.md` line 104 — `impl Stream` in trait method (object-unsafe) |
| `ApprovalLevel` — non-canonical variants | **2** | ⚠️ `26_Test_Strategy.md` line 60 (`::AskEveryTime`), `06_Capability_Matrix.md` line 33 (`::AlwaysApprove`) |
| `AgentEvent` — non-struct-variant usage | **3** | ⚠️ See §Cross-File below |
| `Arc<Box` | **0** (comment only) | Same note-comment as above ✅ |

### Flagged `AlwaysAsk|AskForDangerous|…` Occurrences (detail)

| File | Line | Content | Verdict |
|---|---|---|---|
| `20_Security_Model.md` | 60, 62 | `ToolRisk` variant comments mentioning `AskForDangerous mode` | ⚠️ Prose references old `ApprovalMode` name |
| `18_Config_System.md` | 146 | `ApprovalMode::AskForDangerous` | ⚠️ `ApprovalMode` is a **separate enum** from `ApprovalLevel` — verify it's intentional and documented |
| `26_Test_Strategy.md` | 60 | `ApprovalLevel::AskEveryTime` | ❌ **Invalid variant** — should be `Required` |
| `01_Analysis/03_Hermes_Agent_Feature_Audit.md` | 67 | `AlwaysAsk / AskForDangerous / AlwaysApprove` | ℹ️ Historical analysis doc — acceptable |
| `06_Capability_Matrix.md` | 33 | `ApprovalLevel::AlwaysApprove` | ❌ **Invalid variant** — should be `Safe` |

---

## Core File Validation

| File | Internal Consistent? | Issues |
|---|---|---|
| `17_Approval_Membrane.md` | ✅ **Yes** | All 4 canonical variants (`Safe/Confirmation/Required/Blocked`) used correctly. Match arms are valid and exhaustive with `_` fallthrough. `ApprovalMembrane` struct fields consistent with usage. One minor code note: `response_tx: /* ... */` placeholder in `ask_user()` is incomplete but intentional documentation shorthand. |
| `20_Security_Model.md` | ⚠️ **Mostly** | `ApprovalLevel` enum redeclared inline with correct canonical variants. However, `ToolRisk` variants here (`ReadOnly/SafeWrite/Dangerous/Irreversible/NetworkWrite`) diverge from `17_Approval_Membrane.md` definition (`ReadOnly/Network/LocalWrite/Destructive`). **Two different ToolRisk definitions across docs.** Match arm `(ApprovalLevel::Required, risk) if risk >= ToolRisk::Dangerous` uses `PartialOrd` on `ToolRisk`, which requires `Ord` derive — not shown in this file's definition. |
| `29_Agent_Loop_Implementation.md` | ⚠️ **Mostly** | `AgentEvent` definition here uses tuple-style `AssistantMessage(String)` and `FinalResponse(String)` variants, but the canonical definition in `13_Core_Agent_Loop_Design.md` and `31_Streaming_And_Realtime_Output.md` uses named-field struct variants (`TextDelta { content }`, `TextComplete { content }`, etc.). **The two AgentEvent definitions are inconsistent.** The enum in §2 of this file doesn't include `TextDelta`, `ToolCallStart`, `Done`, `Error` variants present in the canonical version. |
| `12_Workspace_And_Crate_Structure.md` | ✅ **Yes** | `LlmProvider` trait correctly shows `Pin<Box<dyn Stream<...>>>` return. `Tool` trait signature consistent. No orphaned types. Crate layout is self-consistent. |
| `16_Tool_System_Architecture.md` | ⚠️ **Minor** | `ToolRegistry` correctly uses `Arc<dyn Tool>`. `Tool` trait here uses `fn schema()` returning `serde_json::Value`, while `12_Workspace_And_Crate_Structure.md` uses `fn parameters()` returning `RootSchema`. **Method name divergence: `schema()` vs `parameters()`.** `TerminalTool` impl in §5 calls `fn parameters()` and `fn risk()` — but the trait definition in §1 declares `fn schema()` and `fn approval_level()`. Internal inconsistency within same file. |

---

## Cross-File Type Propagation

### `ApprovalLevel` — 5 sampled files

| File | Variants Used | Canonical? |
|---|---|---|
| `17_Approval_Membrane.md` | `Safe`, `Confirmation`, `Required`, `Blocked` | ✅ |
| `16_Tool_System_Architecture.md` | `Safe`, `Confirmation`, `Required`, `Blocked` | ✅ |
| `20_Security_Model.md` | `Safe`, `Confirmation`, `Required`, `Blocked` | ✅ |
| `30_Tool_Execution_Engine.md` | `Safe`, `Confirmation` | ✅ |
| `26_Test_Strategy.md` | `AskEveryTime` | ❌ **Invalid variant — should be `Required`** |

### `AgentEvent` — 3 sampled files

| File | Style Used | Struct-variant form? | Issues |
|---|---|---|---|
| `13_Core_Agent_Loop_Design.md` | `TextDelta { content }`, `TextComplete { content }`, `ToolCallStart { id, name }`, etc. | ✅ Canonical | — |
| `31_Streaming_And_Realtime_Output.md` | `TextDelta { content }`, `TextComplete { content }`, `ToolCallStart { id, name }` | ✅ Canonical | `TextDelta { .. }` used in match arms ✅ |
| `29_Agent_Loop_Implementation.md` | `AssistantMessage(String)`, `FinalResponse(String)`, `ToolCallStarted { tool_name, call_id }` | ❌ **Mixed** | Tuple variants `AssistantMessage(String)` and `FinalResponse(String)` contradict canonical struct-variant-only definition; also emits `ToolCallStarted` (with `_ed` suffix) while canonical uses `ToolCallStart`. |

**Bonus — divergent usage in `36_TUI_Implementation.md`:**
- Uses `AgentEvent::TextDelta(chunk)` — **tuple style** (old form)
- Uses `AgentEvent::ToolCall { name, .. }` — not present in canonical definition (should be `ToolCallStart`)
- Uses `AgentEvent::Done { .. }` — not in `29_Agent_Loop_Implementation.md` definition

**Bonus — `52_Stream_Processing.md`:**
- `AgentEvent::TextDelta(chunk)` — tuple-style, inconsistent with canonical `TextDelta { content }`

---

## Issues Summary

| Severity | Count | Description |
|---|---|---|
| ❌ Critical | 2 | `ApprovalLevel::AskEveryTime` and `AlwaysApprove` — invalid variants in non-analysis docs |
| ❌ Critical | 1 | `impl Stream` in trait method signature in `22_TypeScript_To_Rust_Patterns.md` — object-unsafe |
| ❌ Critical | 3 | `AgentEvent` variant name divergence: `ToolCallStarted` vs `ToolCallStart`, `AssistantMessage` vs `TextDelta`, tuple vs struct style |
| ⚠️ Major | 2 | `Tool` trait method name divergence: `schema()` vs `parameters()`, `approval_level()` vs `risk()` |
| ⚠️ Major | 1 | `ToolRisk` enum defined with different variants in `17_Approval_Membrane.md` vs `20_Security_Model.md` |
| ℹ️ Minor | 2 | Historical analysis docs (`01_Analysis/`) use old enum names — acceptable as audit artifacts |

---

## Consistency Score: 6.5/10

**Breakdown:**
- No `ToolOutput` ✅ (+1)
- No live `Arc<Box<dyn Tool>>` ✅ (+1)
- `Pin<Box<dyn Stream>>` in canonical trait ✅ (+1)
- `ApprovalLevel` variants mostly correct, 2 violations ⚠️ (+0.5)
- `AgentEvent` struct-variant form in canonical files but scattered tuple-style usage in consumers ❌ (-1.5)
- `Tool` trait method name inconsistency across files ❌ (-1)
- `ToolRisk` enum definition split across 2 incompatible definitions ❌ (-0.5)
- `impl Stream` in trait signature in migration doc ❌ (-0.5)

---

## Verdict

⚠️ **Needs one more targeted pass before using as build reference.**

The canonical architecture files (`17_Approval_Membrane.md`, `16_Tool_System_Architecture.md`, `12_Workspace_And_Crate_Structure.md`) are largely clean. The critical failures are concentrated in:

1. **`29_Agent_Loop_Implementation.md`** — must align `AgentEvent` to canonical struct-variant form and fix variant names (`ToolCallStarted` → `ToolCallStart`, `AssistantMessage` → `TextDelta`/`TextComplete`)
2. **`36_TUI_Implementation.md`** — must update all `AgentEvent` match arms to struct-variant canonical form
3. **`26_Test_Strategy.md`** — `ApprovalLevel::AskEveryTime` → `ApprovalLevel::Required`
4. **`06_Capability_Matrix.md`** — `ApprovalLevel::AlwaysApprove` → `ApprovalLevel::Safe`
5. **`22_TypeScript_To_Rust_Patterns.md`** — `impl Stream` in trait method → `Pin<Box<dyn Stream<...> + Send>>`
6. **`20_Security_Model.md`** — reconcile `ToolRisk` variant names with `17_Approval_Membrane.md`
7. **`16_Tool_System_Architecture.md`** — reconcile `fn schema()` / `fn approval_level()` vs `fn parameters()` / `fn risk()` within same file

**Recommended: Pass 4 = targeted corrections to the 7 files above.**
