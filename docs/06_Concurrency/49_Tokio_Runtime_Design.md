# Tokio Runtime Design

> **Status:** ✅ Complete
> **Category:** Concurrency

---

## 1. Why Tokio

Talon is I/O-bound: it waits on LLM HTTP calls, tool subprocesses,
database queries, and gateway events. CPU-bound work is minimal
(JSON parsing, template rendering). Tokio's multi-threaded runtime is
the correct choice — no compute threads needed.

| Alternative | Why Not |
|-------------|---------|
| `async-std` | Smaller ecosystem; tokio dominates in 2024 |
| `smol` | Great for embedded, overkill simplicity for us |
| Threads only | 10-50x memory overhead per connection |
| Single-thread | Wasted cores; approval prompts block everything |

---

## 2. Runtime Configuration

```rust
// src/main.rs
fn main() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)          // Default: number of CPU cores
        .enable_all()               // io, time, signal
        .thread_name("talon-worker")
        .on_thread_start(|| {
            tracing::debug!(thread = ?std::thread::current().id(), "worker started");
        })
        .build()?;

    runtime.block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    // Load config, init tracing, start gateways...
    let config = Config::load()?;
    init_tracing(&config.log_level);

    let app = AppState::new(config).await?;
    app.run().await
}
```

For single-user CLI usage, `worker_threads(4)` is generous — reduce to 2
for low-resource devices. Cron jobs (isolated processes) use
`new_current_thread()` for minimal overhead.

---

## 3. Task Hierarchy

```
main task (AppState::run)
├── Gateway listener tasks (one per gateway)
│   ├── telegram_listener
│   ├── discord_listener (optional)
│   └── http_listener (optional)
├── Agent run tasks (one per concurrent session)
│   ├── agent_loop (main conversation loop)
│   │   ├── llm_stream (HTTP SSE reader)
│   │   └── tool_tasks (per tool call, with join_all)
│   └── event_delivery tasks
├── Cron scheduler task
└── Health check task
```

---

## 4. Task Spawning Patterns

### 4.1 Independent background tasks
For things that run forever (gateway listeners, [cron scheduler](../04_Core_Features/33_Cron_Scheduler.md)):

```rust
// JoinHandle stored in AppState for graceful shutdown
let gateway_handle = tokio::spawn(async move {
    telegram_gateway.listen(input_tx).await
        .context("Telegram gateway crashed")?;
    Ok::<_, anyhow::Error>(())
});
```

### 4.2 Scoped tasks (bounded lifetime)
For agent runs — must complete before session is considered done:

```rust
// Use JoinSet to track all active agent tasks
let mut join_set: JoinSet<Result<(), AgentError>> = JoinSet::new();

join_set.spawn(async move {
    agent_loop.run(input, output_tx).await
});

// Wait for all to finish on shutdown
while let Some(result) = join_set.join_next().await {
    if let Err(e) = result.flatten() {
        tracing::error!("Agent task failed: {e}");
    }
}
```

### 4.3 Parallel tool calls
When the LLM requests multiple tools in one response:

```rust
let tool_futures: Vec<_> = tool_calls.iter().map(|tc| {
    let tool = tool_registry.get(&tc.name).unwrap().clone();
    let args = tc.args.clone();
    tokio::spawn(async move {
        tool.execute(args).await
    })
}).collect();

let results = futures::future::join_all(tool_futures).await;
```

---

## 5. Blocking Operations

CPU-heavy or blocking-I/O code must not run on Tokio worker threads.
Talon uses `spawn_blocking` for:

- `[rusqlite](../07_Memory_System/55_SQLite_FTS5_In_Rust.md)` queries (synchronous C library)
- Regex compilation
- [WASM plugin](../02_Architecture/17_Plugin_And_Skill_Architecture.md) execution (CPU-intensive)
- Image processing

```rust
pub async fn db_query<T, F>(conn: Arc<Mutex<Connection>>, f: F) -> Result<T>
where
    F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let conn = conn.lock().unwrap();
        f(&conn)
    })
    .await
    .map_err(|e| DbError::JoinError(e.to_string()))?
}
```

---

## 6. Graceful Shutdown

```rust
pub async fn run_until_signal(app: Arc<AppState>) -> anyhow::Result<()> {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(
        tokio::signal::unix::SignalKind::terminate()
    )?;

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Received Ctrl+C, shutting down…");
        }
        #[cfg(unix)]
        _ = sigterm.recv() => {
            tracing::info!("Received SIGTERM, shutting down…");
        }
    }

    // Signal all tasks to stop
    app.shutdown_token.cancel();

    // Wait for active agent tasks (max 10s)
    tokio::time::timeout(
        Duration::from_secs(10),
        app.join_active_tasks(),
    ).await.ok();

    // Flush pending deliveries
    app.flush_gateways().await;

    tracing::info!("Shutdown complete");
    Ok(())
}
```

---

## 7. Performance Characteristics

| Scenario | Latency | Memory |
|----------|---------|--------|
| Idle (no active tasks) | 0 | ~8 MB |
| Single LLM stream | <5ms overhead | ~2 MB/session |
| 5 concurrent sessions | Linear scaling | ~10 MB total |
| 10 parallel tool calls | join_all adds <1ms | ~500KB each |
| Cron job (isolated process) | Startup ~50ms | ~5 MB |

Rust's zero-cost async means Talon's overhead is primarily from
the underlying I/O (LLM calls, subprocesses) — not the runtime.
---

## Related Documents

### Depends On
- [Cargo Workspace Design](../02_Architecture/12_Workspace_And_Crate_Structure.md)

### Used By
- [Subagent & Delegation Architecture](../02_Architecture/19_Subagent_And_Delegation_Architecture.md)
- [Cron Scheduler](../04_Core_Features/33_Cron_Scheduler.md)

### See Also
- [Async Tool Execution](50_Async_Tool_Execution.md)
- [Channel Patterns](51_Channel_Patterns.md)
- [Tokio Runtime Config](50a_Tokio_Runtime_Config.md)

