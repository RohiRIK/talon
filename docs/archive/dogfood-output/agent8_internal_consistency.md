# Agent 8 — Internal Architecture Consistency Audit

> **Note on file paths:** The task specified doc numbers 11–13, 16 in `02_Architecture/` and 30–31 in `04_Core_Features/` that do not exist at those paths. The actual Architecture docs are numbered differently (11–16 map to `11_System_Architecture_Overview`, `12_Workspace_And_Crate_Structure`, `13_Core_Agent_Loop_Design`, `14_State_Machine_And_Lifecycle`, `16_Tool_System_Architecture`). The two `04_Core_Features/` docs (`30_Tool_Execution_Engine.md`, `31_Streaming_And_Realtime_Output.md`) **do** exist. The audit covers all six closest-matching docs.

---

## Clean (no issues)

- `14_State_Machine_And_Lifecycle.md` — State enum, transition table, `SessionSource`, `TurnEvent`, and graceful shutdown are internally self-consistent and syntactically valid Rust.
- `12_Workspace_And_Crate_Structure.md` — Workspace `Cargo.toml` and directory layout are coherent; dependency versions look realistic.

---

## Issues Found

### Issue 1 — `Tool` trait defined three different ways (CRITICAL)

Three docs define the `Tool` trait with incompatible signatures:

| Doc | `execute` signature | Schema method |
|---|---|---|
| `12_Workspace_And_Crate_Structure.md` | `async fn execute(&self, ctx: ToolContext) -> Result<ToolOutput, ToolError>` | `fn parameters(&self) -> RootSchema` |
| `16_Tool_System_Architecture.md` | `async fn execute(&self, ctx: ToolContext) -> Result<ToolOutput, ToolError>` | `fn parameters(&self) -> RootSchema` |
| `30_Tool_Execution_Engine.md` | `async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> ToolResult` | `fn schema(&self) -> serde_json::Value` |

Docs 12 and 16 agree, but doc 30 uses a completely different trait. The argument separation (`args` vs embedded in `ctx`), the `&ToolContext` vs `ToolContext` (borrowed vs moved), and the return type (`Result<ToolOutput, ToolError>` vs flat `ToolResult`) are all incompatible. Only one can be the real implementation.

---

### Issue 2 — `ToolResult` vs `Result<ToolOutput, ToolError>` (CRITICAL)

- **Doc 30** defines `ToolResult { output: String, is_error: bool }` as a flat struct with `ToolResult::success()` / `ToolResult::error()` constructors.
- **Doc 16** uses `Result<ToolOutput, ToolError>` where `ToolOutput { content: Vec<ContentBlock>, is_error: bool }` supports multi-modal content (text + images).

The `ToolOutput` type in doc 16 is richer (supports image blocks) and matches the Anthropic tool-use format. Doc 30's flat string `ToolResult` loses the multi-modal capability entirely. The two cannot coexist — any tool implementation written against one API would fail to compile against the other.

---

### Issue 3 — `ToolContext` defined two incompatible ways (CRITICAL)

- **Doc 30**: `ToolContext { session_id, workdir, profile: Arc<Profile>, approval_tx: Option<mpsc::Sender<ApprovalRequest>> }`
- **Doc 16**: `ToolContext { call_id, session_id, args, agent_config: Arc<AgentConfig>, memory: Arc<MemoryStore>, event_tx: broadcast::Sender<AgentEvent>, profile_dir }`

These are structurally different. Doc 30 references `Arc<Profile>` — a type not defined anywhere in the audited docs. Doc 16's version is the more complete and consistent one (it matches how `dispatch_tool_calls` in doc 13 constructs `ToolContext`).

---

### Issue 4 — `AgentEvent` enum defined twice, incompatibly (CRITICAL)

- **Doc 13 (`13_Core_Agent_Loop_Design.md`)**: `AgentEvent { TurnStarted, LlmDelta, ToolCallStarted, ToolCallCompleted, ToolCallError, TurnCompleted, MemoryUpdated, LimitReached }`
- **Doc 31 (`31_Streaming_And_Realtime_Output.md`)**: `AgentEvent { TextDelta, TextComplete, ToolCallStart, ToolCallArgs, ToolCallComplete, ToolResult, ApprovalRequired, ApprovalDecision, IterationStart, Done, Error }`

These are entirely different enums with different variant names, different field names, different serialization attributes (`#[serde(tag = "type")]` only in doc 31). Only one can exist. Doc 31's version is more detailed and streaming-oriented; doc 13's is more lifecycle-event-oriented. They serve different purposes and should probably be two separate enums, but both are called `AgentEvent`.

---

### Issue 5 — `ToolRegistry` uses `Arc<Box<dyn Tool>>` in doc 30 (BUG)

Doc 30:
```rust
tools: HashMap<String, Arc<Box<dyn Tool>>>,
```

Doc 16 (correct):
```rust
tools: HashMap<String, Arc<dyn Tool>>,
```

`Arc<Box<dyn Tool>>` is a redundant double-indirection — `Box<dyn Tool>` inside `Arc` is unnecessary and unusual. This is not a compile error but is architecturally wrong and inconsistent with doc 16. The `as_ref().as_ref()` call in doc 30's `validate_args` (`tool.as_ref().as_ref()`) reveals the author was working around the double wrapping.

---

### Issue 6 — `ApprovalLevel` conflates two separate concepts (DESIGN BUG)

- **Doc 13**: `ApprovalLevel { AlwaysAsk, AlwaysApprove, AskForDangerous }` — this is the *agent's policy*, stored in `AgentConfig`.
- **Doc 30**: `ApprovalLevel { Safe, Confirmation, Required, Blocked }` — this is used as a *per-tool risk level* (same role as `ToolRisk` in doc 16).
- **Doc 16**: `ToolRisk { ReadOnly, SafeWrite, Dangerous, Irreversible }` — yet another risk taxonomy for tools.

Three different enums covering overlapping concepts, none of them consistent with each other. Doc 30's `ApprovalLevel` is used as `tool.approval_level()`, but doc 16's `Tool` trait exposes `fn risk(&self) -> ToolRisk`. The `ApprovalMembrane` in doc 13 takes an `ApprovalLevel` from config; doc 30's executor checks `tool.approval_level() >= ApprovalLevel::Confirmation`. These cannot coexist as written.

---

### Issue 7 — `impl Trait` in `LlmProvider` trait makes it non-object-safe (WON'T COMPILE)

Doc 12 defines:
```rust
pub trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        req: CompletionRequest,
    ) -> Result<impl Stream<Item = Result<Delta, LlmError>>, LlmError>;
}
```

Using `impl Stream` as a return type in a trait method (RPITIT) makes the trait **non-object-safe**. However, doc 13's `Agent` struct holds `pub llm: Arc<dyn LlmProvider>` — a trait object. This will not compile. To make it work, the return type must either use `Box<dyn Stream<...> + Send>` (object-safe) or the agent must be generic over `L: LlmProvider`.

---

### Issue 8 — `WasmTool::name()` returns `&'static str` from a `String` field (WON'T COMPILE)

Doc 16:
```rust
impl Tool for WasmTool {
    fn name(&self) -> &'static str { &self.name }  // self.name is String
```

`&self.name` has lifetime `'self`, not `'static`. This is a lifetime error — the compiler will reject it. The `Tool` trait requires `&'static str` but `WasmTool`'s name is a runtime `String`. The trait definition itself is the problem: `fn name(&self) -> &'static str` cannot be satisfied by any dynamically-constructed tool. It should be `fn name(&self) -> &str`.

---

### Issue 9 — `ToolExecutor::execute_parallel` calls `self.clone()` without `Clone` derived (WON'T COMPILE)

Doc 30:
```rust
let executor = self.clone();
```

`ToolExecutor` is never shown to derive or implement `Clone`. Without it, this line will not compile. Since `ToolExecutor` holds `Arc<ToolRegistry>` and `Arc<ApprovalService>`, a `Clone` derive would work — but it's missing from the docs.

---

### Issue 10 — `execute_with_streaming` uses `?` with wrong error type (WON'T COMPILE)

Doc 31:
```rust
pub async fn execute_with_streaming(...) -> ToolResult {
    let mut child = tokio::process::Command::new("sh")
        ...
        .spawn()
        .map_err(|e| ToolResult::error(e.to_string()))?;
```

The function returns `ToolResult` (not `Result<_, ToolResult>`). The `?` operator requires the function to return `Result<_, E>` where `E` matches the mapped error type. Using `?` to propagate `ToolResult` from a function that returns `ToolResult` directly is not valid Rust. This should use `match` or an early return: `let mut child = match ... { Ok(c) => c, Err(e) => return ToolResult::error(...) };`.

---

### Issue 11 — `blocking_lock()` called inside async context under async lock (POTENTIAL DEADLOCK)

Doc 14, `SessionManager::gc()`:
```rust
pub async fn gc(&self) {
    ...
    let mut sessions = self.sessions.write().await;  // async write lock held
    sessions.retain(|_, s| {
        let s = s.blocking_lock();  // blocking lock inside async closure
```

`blocking_lock()` on a `tokio::sync::Mutex` panics if called from within a Tokio async context. The `retain` closure runs synchronously inside an async fn while an async write-lock is held. This is both a potential panic and a potential deadlock. Should use `try_lock()` or restructure with an async approach.

---

### Issue 12 — `EventSource` POST is not a real browser API (FABRICATED CODE)

Doc 31's JavaScript client example:
```javascript
const es = new EventSource('/chat/stream', { method: 'POST', body: JSON.stringify({ message }) });
```

The browser's `EventSource` API does not support `method` or `body` options — it only makes GET requests. This would silently fall back to a GET with no body. The correct approach is to use `fetch` with a `ReadableStream` response, or a library like `eventsource-fetch`. This is a realistic mistake, but fabricated as working code.

---

### Issue 13 — `ToolResult::schema_error()` and `ToolResult::denied()` referenced but not defined

Doc 30's lifecycle diagram mentions:
```
[Schema Validation] ──fail──► ToolResult::schema_error()
[Approval Gate] ──denied──► ToolResult::denied()
```

But the `ToolResult` impl block only defines `success()` and `error()`. Neither `schema_error()` nor `denied()` are implemented. The actual `execute_call` code in the same doc uses `ToolResult::error(...)` for both cases, which works, but the diagram references non-existent constructors.

---

### Issue 14 — `AgentConfig::context_window_turns` field missing

Doc 13 references `self.agent.config.context_window_turns` in `ContextBuilder::build()`:
```rust
let history = self.agent.session
    .load_recent(self.session_id, self.agent.config.context_window_turns)
    .await?;
```

But the `AgentConfig` struct definition in doc 13 does not include a `context_window_turns` field:
```rust
pub struct AgentConfig {
    pub model: String,
    pub system_prompt: String,
    pub max_iterations: u32,
    pub max_tokens: u32,
    pub approval_level: ApprovalLevel,
    pub tools_enabled: Vec<String>,
    pub profile_dir: PathBuf,
}
```

This field is referenced but not declared — a compile error.

---

## Verdict

**Score: 2 / 5**

The docs demonstrate a genuine understanding of Rust async architecture patterns (tokio, broadcast channels, Arc/Mutex, async-trait, state machines). The high-level design is sound and the system diagram is coherent. However, there are **4 show-stopping compile errors** (Issues 7, 8, 10, 11), **3 critical cross-doc type conflicts** that mean no coherent implementation could satisfy all docs simultaneously (Issues 1, 2, 4), and **2 design inconsistencies** in the approval/risk taxonomy. The docs appear to have been authored in passes where later docs (30, 31) diverged from the canonical types established in docs 12, 13, 16 without updating them. A reconciliation pass is needed before any implementation can begin.

**Priority fixes:**
1. Canonicalize `Tool` trait, `ToolResult`/`ToolOutput`, and `ToolContext` to one definition (recommend doc 16's version)
2. Merge or rename the two `AgentEvent` enums
3. Fix `LlmProvider` to be object-safe (`Box<dyn Stream>` return)
4. Change `Tool::name()` return type to `&str` (not `&'static str`)
5. Fix the three `ApprovalLevel`/`ToolRisk` enums into a consistent two-enum design (agent policy + tool risk)
