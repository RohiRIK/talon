# Error Handling Strategy

> **Status:** ✅ Complete
> **Category:** Concurrency
> **Last corrected:** dogfood pass 3

---

## 1. The Three Audiences

Every error in Talon has three representations:

| Audience | Format | Example |
|----------|--------|---------|
| **User** | Clean, actionable English sentence | "Couldn't read file — check the path exists." |
| **Developer** | Structured JSON log with context | `{"level":"ERROR","error":"IoError","path":"/foo","errno":2}` |
| **LLM** | Tool error content block | `ToolResult { is_error: true, output: "File not found: /foo".into(), metadata: None }` |

---

## 2. Error Handling Split: Libraries vs. Binary

| Crate | Crate type | Error tool | Why |
|-------|-----------|-----------|-----|
| `talon-core`, `talon-llm`, `talon-tools`, `talon-memory`, `talon-gateway`, `talon-plugins` | Library | `thiserror` typed enums | Callers pattern-match on variants |
| `talon` (binary) | Application | `anyhow::Result` + `.context()` | Human-readable error chains; no caller to pattern-match |

```rust
// talon/src/main.rs — binary uses anyhow
use anyhow::{Context, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let config = talon_core::Config::load("~/.talon/config.toml")
        .context("failed to load config")?;
    talon_core::run(config).await.context("agent loop exited with error")?;
    Ok(())
}
```

---

## 3. Crate-Level Error Types

```rust
// talon-llm/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("rate limited — retry after {retry_after}s")]
    RateLimit { retry_after: u64 },

    #[error("API returned {status}: {message}")]
    ApiError { status: u16, message: String },

    #[error("context length exceeded: {tokens} > {limit}")]
    ContextTooLong { tokens: u32, limit: u32 },

    #[error("stream truncated")]
    StreamTruncated,

    #[error(transparent)]
    Network(#[from] reqwest::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

// talon-tools/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("unknown tool: {0}")]
    UnknownTool(String),

    #[error("missing required argument: {0}")]
    MissingArg(String),

    #[error("invalid argument {key}: {reason}")]
    InvalidArg { key: String, reason: String },

    #[error("execution timed out after {seconds}s")]
    Timeout { seconds: u64 },

    #[error("permission denied by approval membrane")]
    Denied,

    #[error("process exited with code {code}: {stderr}")]
    ProcessFailed { code: i32, stderr: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

// talon-core/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error(transparent)]
    Llm(#[from] LlmError),

    #[error(transparent)]
    Tool(#[from] ToolError),

    #[error(transparent)]
    Memory(#[from] MemoryError),

    #[error("iteration limit reached ({0})")]
    IterationLimit(u32),

    #[error("token budget exhausted")]
    TokenBudgetExhausted,

    #[error("session not found: {0}")]
    SessionNotFound(Uuid),
}
```

---

## 4. Rate Limit Auto-Retry

```rust
pub async fn with_retry<F, T, E>(
    mut f: impl FnMut() -> F,
    max_attempts: u32,
) -> Result<T, E>
where
    F: Future<Output = Result<T, E>>,
    E: IsRateLimit,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) if attempt < max_attempts => {
                if let Some(retry_after) = e.retry_after_secs() {
                    let wait = Duration::from_secs(retry_after)
                        + Duration::from_millis(rand::random::<u64>() % 1000);
                    tracing::warn!(attempt, retry_after, "rate limited, backing off");
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                } else {
                    return Err(e);
                }
            }
            Err(e) => return Err(e),
        }
    }
}
```

---

## 5. Panic Policy

Talon must **never panic** in the hot path. Rules:

| Construct | Allowed? | Alternative |
|-----------|----------|-------------|
| `unwrap()` on `None` | ❌ No | `ok_or(...)? ` |
| `expect()` on `None` | ⚠️ Init only | Document why it's safe |
| `unwrap()` on `Err` | ❌ No | `?` |
| `panic!()` | ❌ No in production | `return Err(...)` |
| Array index `[n]` | ⚠️ Bounds checked | `get(n).ok_or(...)` |

CI lint rule: `cargo clippy -- -D clippy::unwrap_used -D clippy::expect_used` (except in `#[cfg(test)]` and `fn main()`).

---

## 6. Structured Logging

```rust
// In tools
tracing::info!(
    tool = %ctx.call_id,
    tool_name = "terminal",
    command = %command,
    "tool call started"
);

tracing::error!(
    tool = %ctx.call_id,
    exit_code = code,
    stderr = %stderr,
    "tool process failed"
);
```

Subscriber configuration:

```rust
tracing_subscriber::registry()
    .with(tracing_subscriber::fmt::layer()
        .json()
        .with_target(true)
        .with_thread_ids(true))
    .with(tracing_subscriber::EnvFilter::from_default_env())
    .init();
```

`RUST_LOG=talon=debug,reqwest=warn cargo run`

---

## 7. Error-to-ToolResult Mapping

```rust
impl From<ToolError> for ToolResult {
    fn from(e: ToolError) -> Self {
        ToolResult {
            is_error: true,
            output: format!("<tool_error>\n{e}\n</tool_error>"),
            metadata: None,
        }
    }
}
```

The LLM receives the error in its tool_result turn, can reason about it, and retry with corrected arguments.
---

## Related Documents

### Used By
- [Core Agent Loop Design](../02_Architecture/13_Core_Agent_Loop_Design.md)
- [Tool System Architecture](../02_Architecture/16_Tool_System_Architecture.md)
- [LLM Provider Abstraction](../05_API_Bindings/41_LLM_Provider_Abstraction.md)

### See Also
- [Logging & Observability](../08_DevOps/64_Logging_And_Observability.md)
- [Canonical Types](../00_Connections/05_Canonical_Types.md)

