# Rust Type Consistency Audit — Pass 2

> Audited: 2026-05-25
> Scope: All docs under `/home/rohi/homelab/projects/ernest/docs/`
> Focus: Verify correction-pass fixes landed; identify residual and new issues.

---

## Fixed Issues — Confirmed

### ✅ 1. `Arc<Box<dyn Tool>>` → `Arc<dyn Tool>` (partial)
Confirmed fixed in all **6 core architecture files**:
- `16_Tool_System_Architecture.md` line 69: `HashMap<String, Arc<dyn Tool>>` with explicit comment `// NOT Arc<Box<dyn Tool>>`
- `30_Tool_Execution_Engine.md` line 108: `HashMap<String, Arc<dyn Tool>>`
- `17_Plugin_And_Skill_Architecture.md`: registry composition uses `Arc::from(box)` pattern
- `13_Core_Agent_Loop_Design.md`: `tools: Arc<ToolRegistry>` (correct indirection)

### ✅ 2. `ToolResult { output: String, is_error: bool, metadata: Option<Value> }` standardized in core files
Confirmed in all 6 core files:
- `16_Tool_System_Architecture.md` lines 32–36: canonical struct definition present
- `30_Tool_Execution_Engine.md` lines 78–91: canonical struct + `success()`/`error()` constructors
- `17_Plugin_And_Skill_Architecture.md` lines 70, 82: `ToolResult::error()`, `ToolResult::success()` used correctly

### ✅ 3. `WasmPlugin::name()` lifetime fixed
- `17_Plugin_And_Skill_Architecture.md` line 102: `fn name(&self) -> &str { &self.name }` with comment `// &str tied to self lifetime, not &'static str` ✓
- `16_Tool_System_Architecture.md` line 176: same fix with identical comment ✓

### ✅ 4. `LlmProvider` trait split into `complete()` + `stream()` for object safety
- `41_LLM_Provider_Abstraction.md` lines 74–103: Correct. `DeltaStream` type alias defined as `Pin<Box<dyn Stream<...>>>`. Trait has separate `complete()` returning `LlmResponse` and `stream()` returning `DeltaStream`. Comment explains object-safety rationale.

### ✅ 5. `AgentEvent` canonical definition (in primary files)
- `31_Streaming_And_Realtime_Output.md` (canonical source) and `13_Core_Agent_Loop_Design.md` both carry the same full enum with `TextDelta { content: String }` (struct variant), `TextComplete`, `ToolCallStart`, `ToolCallArgs`, `ToolCallComplete`, `ToolResult`, `ApprovalRequired`, `ApprovalDecision`, `IterationStart`, `Done`, `Error`. These two files are consistent.

### ✅ 6. `ApprovalLevel { Safe, Confirmation, Required, Blocked }` in core architecture files
- `13_Core_Agent_Loop_Design.md` lines 174–179: correct canonical enum ✓
- `16_Tool_System_Architecture.md` lines 40–45: correct ✓
- `30_Tool_Execution_Engine.md` lines 94–99: correct ✓

---

## Remaining Issues

### ❌ 1. `Arc<Box<dyn Tool>>` still present in `06_Concurrency/50_Async_Tool_Execution.md`
Two residual occurrences, NOT fixed by the correction pass:
- Line 31: `- \`Sync\`: the \`Arc<Box<dyn Tool>>\` may be accessed from multiple threads`
- Line 129: `tool: Arc<Box<dyn Tool>>,`

**Impact:** Contradicts the canonical pattern established in the 6 core files.

### ❌ 2. `ToolOutput` not replaced — 45 occurrences across 20+ non-core files
The fix only landed in the 6 targeted core architecture files. Extensive use of `ToolOutput` (and `ToolOutput::text()`, `Result<ToolOutput, ToolError>`) remains throughout:

| File | Occurrences |
|------|-------------|
| `06_Concurrency/54_Error_Handling_Strategy.md` | 5 (incl. `From<ToolError> for ToolOutput` impl) |
| `02_Architecture/12_Workspace_And_Crate_Structure.md` | 1 (`async fn execute` signature) |
| `02_Architecture/17_Approval_Membrane.md` | 1 (fn return type) |
| `02_Architecture/19_Subagent_And_Delegation_Architecture.md` | 3 |
| `04_Core_Features/30_Terminal_Tool.md` | 3 |
| `04_Core_Features/31_File_System_Tool.md` | 8 |
| `04_Core_Features/32_Browser_Tool.md` | 6 |
| `04_Core_Features/34_Web_Search_Tool.md` | 4 |
| `04_Core_Features/35_Send_Message_Tool.md` | 3 |
| `04_Core_Features/36_MCP_Client_Tool.md` | 3 |
| `04_Core_Features/37_Subagent_Delegation.md` | 2 |
| `07_Memory_System/58_FTS5_Search_Deep_Dive.md` | 2 |
| `01_Analysis/02_OpenClaw_Feature_Audit.md` | 2 |
| `01_Analysis/03_Hermes_Agent_Feature_Audit.md` | 1 |
| `03_Migration_Strategy/23_Python_To_Rust_Patterns.md` | 1 |
| `08_DevOps/64_Logging_And_Observability.md` | 1 |

These files all use `Result<ToolOutput, ToolError>` as the execute signature — different from the canonical `-> ToolResult` (infallible, error embedded in struct).

### ❌ 3. `ApprovalLevel` — THREE conflicting enum definitions remain
The canonical `{ Safe, Confirmation, Required, Blocked }` fix landed in the 6 core files, but two other files were **not updated**:

**`02_Architecture/17_Approval_Membrane.md`** (lines 44–51):
```rust
pub enum ApprovalLevel {
    AlwaysAsk,
    AskForDangerous,
    AlwaysApprove,
}
```
3-variant set, completely different names. Used by `always_approve`/`always_ask`/`never_allow` list config.

**`02_Architecture/20_Security_Model.md`** (lines 44–49):
```rust
pub enum ApprovalLevel {
    AlwaysApprove,
    ApproveOnce,
    AskEveryTime,
    AlwaysDeny,
}
```
4-variant set, again different names. Used in `26_Test_Strategy.md` (`ApprovalLevel::AskEveryTime`) and `06_Capability_Matrix.md` (`ApprovalLevel::AlwaysApprove`).

**Summary:** Three incompatible `ApprovalLevel` definitions now exist across the docs. The correction pass only unified the core 3 files.

### ❌ 4. `AgentEvent` diverges in `04_Core_Features/29_Agent_Loop_Implementation.md`
Lines 58–65 define a **stripped-down, tuple-variant** enum that conflicts with the canonical struct-variant enum:

```rust
// In 29_Agent_Loop_Implementation.md (DIVERGENT):
pub enum AgentEvent {
    TextDelta(String),                               // tuple, not struct
    ToolCall { name: String, id: String },           // missing fields vs canonical ToolCallStart
    ToolResult { id: String, output: String, is_error: bool }, // missing `name`
    ApprovalRequired { tool: String, id: String, args: Value }, // different fields
    Done { final_response: String, iterations: u32 }, // missing `usage`
    Error(AgentError),                              // tuple variant, not struct
}
```
vs canonical in `31_Streaming_And_Realtime_Output.md`:
```rust
TextDelta { content: String },       // struct variant
ToolCallStart { id: String, name: String },
// ... plus ToolCallArgs, ToolCallComplete, IterationStart, UsageSummary in Done, etc.
```

Also, `06_Concurrency/52_Stream_Processing.md` line 205 uses `AgentEvent::TextDelta(chunk)` (tuple-style), and `36_TUI_Implementation.md` line 170 also uses `AgentEvent::TextDelta(chunk)`. These are consistent with the `29_Agent_Loop_Implementation.md` variant but contradict the canonical struct variant.

### ❌ 5. `impl Stream` still used in trait context in `12_Workspace_And_Crate_Structure.md`
Line 189:
```rust
) -> Result<impl Stream<Item = Result<Delta, LlmError>>, LlmError>;
```
This is in a trait definition context, making the trait **not object-safe**. The fix in `41_LLM_Provider_Abstraction.md` uses `DeltaStream = Pin<Box<dyn Stream<...>>>`, but `12_Workspace_And_Crate_Structure.md` was not updated to match.

---

## New Issues Introduced

### ⚠️ 1. `TerminalTool` impl in `16_Tool_System_Architecture.md` has wrong return type for `name()`
Lines 147–156 show a `TerminalTool` implementing `Tool`:
```rust
fn name(&self) -> &'static str { "terminal" }
fn description(&self) -> &'static str { ... }
```
But the canonical `Tool` trait defined just above (lines 18–29) uses `&str` (without `'static`). The correction pass updated the **trait** signature but left the concrete **impl example** using `&'static str`. This is a direct contradiction within the same file — a new inconsistency introduced by the patch.

Note: `&'static str` is technically valid where `&str` is required (it satisfies the lifetime), but the example is pedagogically inconsistent and implies the old (wrong) signature.

### ⚠️ 2. `ToolContext` field inconsistency introduced between files
After the correction pass:
- `16_Tool_System_Architecture.md` (lines 54–61): `ToolContext` has `call_id: String` field
- `30_Tool_Execution_Engine.md` (lines 71–76): `ToolContext` does **not** have `call_id`

The two files now disagree on whether `ToolContext` carries a `call_id`. Also `13_Core_Agent_Loop_Design.md` shows `ToolContext` being constructed **with** `call_id` (line 143) and `event_tx` — neither of which appears in the `30_Tool_Execution_Engine.md` definition.

### ⚠️ 3. `build_tool_registry` in `17_Plugin_And_Skill_Architecture.md` uses `async` illegally
Line 224:
```rust
pub fn build_tool_registry(config: &Config) -> Arc<ToolRegistry> {
    // ...
    for plugin in load_plugins(&config.plugins_dir).await.unwrap_or_default() {
```
The function is declared as `fn` (not `async fn`) but uses `.await` inside. This is a broken code example — it cannot compile. This may be a pre-existing issue or introduced by the patch reorganisation.

### ⚠️ 4. `31_Streaming_And_Realtime_Output.md` — `complete_or_stream` signature is inconsistent with split trait
Lines 83–98 show:
```rust
pub async fn complete_or_stream(
    &self,
    request: LlmRequest,    // ← uses LlmRequest, not CompletionRequest
    event_tx: mpsc::Sender<AgentEvent>,
) -> Result<LlmResponse, LlmError> {
    if self.supports_streaming() {
        self.stream(request, event_tx).await  // ← stream() takes 2 args, but
                                              //   canonical stream() only takes req
```
After the split into `complete()` + `stream()`, `stream()` returns a `DeltaStream` (no `event_tx` param). This helper function was not updated to reflect the new trait signature, creating a broken example.

---

## Summary Table

| Fix | Status | Notes |
|-----|--------|-------|
| `Arc<Box<dyn Tool>>` eliminated | ⚠️ Partial | Fixed in 6 core files; `50_Async_Tool_Execution.md` still has it |
| `ToolResult` standardized | ⚠️ Partial | Core 6 fixed; 20+ peripheral docs still use `ToolOutput` |
| `AgentEvent` unified | ⚠️ Partial | 2 of 3 locations fixed; `29_Agent_Loop_Implementation.md` diverges + TUI/stream docs use tuple variant |
| `ApprovalLevel` standardized | ⚠️ Partial | 3 of 5 locations fixed; `17_Approval_Membrane.md` and `20_Security_Model.md` still have incompatible variants |
| `LlmProvider` trait split | ✅ Fixed | Correct in `41_LLM_Provider_Abstraction.md` |
| `WasmPlugin::name()` lifetime | ✅ Fixed | Correct in both files |
| `impl Stream` in trait | ⚠️ Partial | Fixed in `41_LLM_Provider_Abstraction.md`; `12_Workspace_And_Crate_Structure.md` still wrong |

---

## Consistency Score: 5/10

**Rationale:**
- The 6 core architecture files are now internally consistent and well-annotated (+3).
- `LlmProvider` split and `WasmPlugin` lifetime are clean fixes (+1).
- `ToolOutput` proliferation across ~20 non-core files is the biggest residual problem (-2).
- `ApprovalLevel` has 3 incompatible definitions across the doc set (-1).
- `AgentEvent` diverges in the agent loop implementation doc and TUI doc (-1).
- 3 new code-correctness issues introduced by the patches (async/fn mismatch, `stream()` signature, `TerminalTool` impl) (-1).
- The fixes are real and meaningful, but scope was too narrow — only the 6 core files were cleaned, leaving the broader doc corpus inconsistent.
