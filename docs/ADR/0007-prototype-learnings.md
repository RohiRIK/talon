# ADR 0007 — Phase 0.5 Prototype Learnings

**Date:** 2026-05-27
**Status:** Accepted
**Phase:** 0.5 → 1

---

## Context

Phase 0.5 built a thin end-to-end agent prototype entirely inside `talon/src/main.rs` to validate
the 7 load-bearing type shapes before they get locked into their home crates in Phase 1. This ADR
records what the prototype revealed.

---

## Learnings

### 1. Enum dispatch vs `Arc<dyn Tool>` for the prototype

**What happened:** Used a `BuiltinTool` enum with concrete `async fn execute()` rather than a
`Tool` trait + `Arc<dyn Tool>` as specified in the 7 load-bearing types.

**Why:** `async fn` in Rust 2024 traits is not automatically dyn-compatible. To use `dyn Tool` you
must either return `Pin<Box<dyn Future<Output = ToolResult> + Send + '_>>` from `execute`, or use
the `dynosaur` crate. Enum dispatch is simpler and zero-overhead for a prototype.

**Decision for Phase 1:** Keep `Arc<dyn Tool>` as the load-bearing type (Type #5), but define the
`Tool` trait with a boxed future return so it is dyn-compatible:

```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    fn approval_level(&self, args: &serde_json::Value) -> ApprovalLevel;
    fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>>;
}
```

Built-in tool impls call a private `async fn` and wrap with `Box::pin(async move { ... })`.

### 2. Anthropic tool-use loop is straightforward

The loop shape (LLM → collect `tool_use` blocks → execute → push `tool_result` user message →
repeat) is clean and requires no special state machine for the prototype. The assistant message
must be pushed into history before processing tool calls — otherwise the API rejects the request
with a role sequence error.

**Decision for Phase 1:** The `AgentState` machine can be simple: `Idle → Thinking → CallingTool
→ Idle | Completed | Failed`. No need for `AwaitingApproval` as a blocking state — approval
should be async and driven via `AgentEvent::ApprovalRequested`.

### 3. `serde` tagged union for `ContentBlock`

`#[serde(tag = "type", rename_all = "snake_case")]` correctly round-trips `ToolUse` ↔
`"type": "tool_use"` and `Text` ↔ `"type": "text"`. No issues. Phase 1 can use the same pattern
for `LlmResponse` content blocks.

### 4. Approval membrane is sync, not async

`check_approval()` reads stdin synchronously. This is fine for the CLI gateway but wrong for
Telegram/HTTP gateways. Phase 1 must use `oneshot::Sender<ApprovalDecision>` inside
`AgentEvent::ApprovalRequested` so each gateway can implement approval in its own way.

### 5. API key lookup order

`TALON_LLM_API_KEY` env var → OS keychain (`keyring` crate). This order is correct for all
environments (CI uses env var; users use keychain after `talon init`).

### 6. Model selection

Default `claude-haiku-4-5-20251001` is appropriate for development and testing. Production users
should set `TALON_LLM_MODEL` or configure the model in `~/.talon/config.toml`. Phase 1 will wire
the config file; for now the env var override is sufficient.

### 7. `reqwest::Client` should be shared

The prototype creates a new `Client` per agent run. Phase 1 should create one `Client` and share
it for the lifetime of the process (pass via `Arc` or store in `Agent`). `reqwest::Client` is
already `Clone` and manages a connection pool internally.

---

## Decisions for Phase 1 (locked)

| # | Type | Decision |
|---|------|----------|
| 1 | `ToolResult` | Keep struct: `{ content: String, is_error: bool }` |
| 2 | `Tool` trait | Add `ToolContext` param; return `Pin<Box<dyn Future>>` for dyn-compatibility |
| 3 | `Database` | `deadpool_sqlite::Pool` wrapper; all ops via `.interact()` |
| 4 | `LlmProvider` trait | Same pattern: `Pin<Box<dyn Future>>` if `dyn` is needed |
| 5 | `Arc<dyn Tool>` | Confirmed as the right type; enum was only for prototype |
| 6 | `ApprovalLevel` | Keep three variants; add `ApprovalMembrane` that emits events |
| 7 | `AgentEvent` | Must include `ApprovalRequested { tx: oneshot::Sender<bool> }` |
