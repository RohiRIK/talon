# Async Tool Execution

> **Status:** ✅ Complete
> **Category:** Concurrency
> **Last corrected:** dogfood pass 3

---

## 1. Overview

Every tool in Talon is async (`async fn execute`) and runs within Tokio.
This means tools can perform I/O (HTTP calls, file reads, subprocess waits)
without blocking the agent loop.

---

## 2. Tool Trait Async Contract

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> ToolResult;
}
```

The `Send + Sync` bounds are required because:
- `Send`: tools may be executed on any Tokio worker thread
- `Sync`: the `Arc<dyn Tool>` may be accessed from multiple threads

---

## 3. Blocking-to-Async Bridge

Some underlying libraries are synchronous ([rusqlite](../07_Memory_System/55_SQLite_FTS5_In_Rust.md), regex, some C bindings).
These use `spawn_blocking` to avoid blocking Tokio threads:

```rust
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let path: PathBuf = args["path"].as_str().unwrap_or("").into();

        // tokio::fs::read_to_string is async and non-blocking:
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => ToolResult::success(content),
            Err(e) => ToolResult::error(format!("Cannot read {:?}: {e}", path)),
        }
    }
}
```

For synchronous operations:
```rust
pub struct DatabaseQueryTool {
    db: Arc<Database>,
}

#[async_trait]
impl Tool for DatabaseQueryTool {
    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult {
        let query = args["query"].as_str().unwrap_or("").to_string();
        let db = self.db.clone();

        // rusqlite is sync — run in blocking thread pool
        let result = tokio::task::spawn_blocking(move || {
            db.execute_query(&query)
        }).await;

        match result {
            Ok(Ok(rows)) => ToolResult::success(format_rows(rows)),
            Ok(Err(e)) => ToolResult::error(e.to_string()),
            Err(e) => ToolResult::error(format!("Thread panic: {e}")),
        }
    }
}
```

---

## 4. Timeout Wrapper

All tool executions are wrapped with a timeout:

```rust
pub async fn execute_with_timeout(
    tool: &dyn Tool,
    args: Value,
    ctx: &ToolContext,
    timeout: Duration,
) -> ToolResult {
    match tokio::time::timeout(timeout, tool.execute(args, ctx)).await {
        Ok(result) => result,
        Err(_elapsed) => ToolResult::error(format!(
            "Tool '{}' timed out after {}s",
            tool.name(),
            timeout.as_secs()
        )),
    }
}
```

Default timeouts per tool type:
- `web_search`: 30s
- `web_extract`: 30s per URL
- `terminal`: 180s (configurable)
- `file_*`: 10s
- `llm_*`: 120s

---

## 5. Cancellation

When the user sends a new message while a tool is running, Talon
cancels the inflight tool:

```rust
pub struct CancellableToolExecution {
    cancel: CancellationToken,
}

impl CancellableToolExecution {
    pub async fn run(
        &self,
        tool: Arc<dyn Tool>,
        args: Value,
        ctx: ToolContext,
    ) -> ToolResult {
        tokio::select! {
            result = tool.execute(args, &ctx) => result,
            _ = self.cancel.cancelled() => {
                ToolResult::error("Tool execution cancelled")
            }
        }
    }
}
```
---

## Related Documents

### Depends On
- [Tool System Architecture](../02_Architecture/16_Tool_System_Architecture.md)
- [Tokio Runtime Design](49_Tokio_Runtime_Design.md)

### See Also
- [Channel Patterns](51_Channel_Patterns.md)
- [Resource Limits & Backpressure](53_Resource_Limits_And_Backpressure.md)
- [Tool Execution Engine](../04_Core_Features/30_Tool_Execution_Engine.md)

