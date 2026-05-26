# Async Migration: Node.js → Tokio

> **Status:** ✅ Complete
> **Category:** Migration Strategy

---

## 1. Mental Model Shift

| Concept | Node.js | Tokio |
|---------|---------|-------|
| Event loop | 1 thread, libuv | N threads, work-stealing |
| Async unit | Promise / callback | `Future` + `async fn` |
| Concurrent tasks | `Promise.all` | `join_all` / `JoinSet` |
| Background task | `setImmediate` | `tokio::spawn` |
| Timer | `setTimeout` | `tokio::time::sleep` |
| Interval | `setInterval` | `tokio::time::interval` |
| Streams | `Readable`, `EventEmitter` | `futures::Stream` |
| Channels | `EventEmitter` | `mpsc`, `broadcast`, `oneshot` |
| Process exit | `process.exit()` | `std::process::exit()` / drop main |

---

## 2. Promise.all → join_all / JoinSet

**Node.js:**
```typescript
const results = await Promise.all(tasks.map(t => runTask(t)));
```

**Rust — static list:**
```rust
let results: Vec<Result<Output, Error>> =
    futures::future::join_all(tasks.iter().map(|t| run_task(t))).await;
```

**Rust — dynamic spawn (JoinSet):**
```rust
let mut set = tokio::task::JoinSet::new();

for task in tasks {
    set.spawn(run_task(task));
}

let mut results = vec![];
while let Some(res) = set.join_next().await {
    results.push(res?);  // propagates panics
}
```

`JoinSet` is preferred when tasks are spawned dynamically or you want results as they complete (not all-at-once).

---

## 3. EventEmitter → tokio::sync Channels

**Node.js pattern (OpenClaw):**
```typescript
class AgentEventBus extends EventEmitter {}
const bus = new AgentEventBus();

bus.on("tool_complete", (data) => sendToTelegram(data));
bus.emit("tool_complete", { id, output });
```

**Problems:** No [backpressure](../06_Concurrency/53_Resource_Limits_And_Backpressure.md), no type safety, listeners accumulate forever.

**Rust — [broadcast channel](../06_Concurrency/51_Channel_Patterns.md) (1 sender, N subscribers):**
```rust
// Create once at startup
let (tx, _rx) = tokio::sync::broadcast::channel::<AgentEvent>(256);

// Subscribe (each gateway creates its own rx)
let mut telegram_rx = tx.subscribe();
let mut discord_rx  = tx.subscribe();

// Emit
tx.send(AgentEvent::ToolCallCompleted { call_id, output }).ok();

// Receive in Telegram task
while let Ok(event) = telegram_rx.recv().await {
    match event {
        AgentEvent::ToolCallCompleted { call_id, output } => {
            telegram.send_message(format!("✅ `{call_id}`: {output}")).await?;
        }
        _ => {}
    }
}
```

**Rust — mpsc channel (N senders, 1 receiver):**
```rust
// For collecting tool results back to the agent loop
let (result_tx, mut result_rx) = tokio::sync::mpsc::channel::<ToolResult>(32);

// Each tool sends its result
tokio::spawn(async move {
    let output = tool.execute(ctx).await;
    result_tx.send(ToolResult { call_id, output }).await.ok();
});

// Agent loop collects all results
let mut results = vec![];
while let Some(r) = result_rx.recv().await {
    results.push(r);
    if results.len() == expected_count { break; }
}
```

---

## 4. Readable Stream → futures::Stream

**Node.js:**
```typescript
for await (const chunk of response.body) {
  process.stdout.write(decoder.decode(chunk));
}
```

**Rust:**
```rust
use futures::StreamExt;

let mut stream = llm.complete(req).await?;
while let Some(delta) = stream.next().await {
    match delta? {
        Delta::Text(t) => print!("{t}"),
        Delta::Done => break,
        _ => {}
    }
}
```

**Transforming streams:**
```rust
// Node.js: stream.pipe(transform).pipe(dest)
// Rust: stream.map(...).filter(...).forward(sink)

let processed = raw_stream
    .filter_map(|item| async move { item.ok() })
    .map(|delta| format_delta(delta))
    .take_while(|s| futures::future::ready(!s.is_empty()));
```

---

## 5. setTimeout / setInterval → Tokio Timers

**Node.js:**
```typescript
const id = setTimeout(() => checkHealth(), 5000);
clearTimeout(id);

const interval = setInterval(() => tick(), 1000);
clearInterval(interval);
```

**Rust — one-shot delay:**
```rust
tokio::time::sleep(Duration::from_secs(5)).await;
check_health().await?;
```

**Rust — cancellable:**
```rust
let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

tokio::spawn(async move {
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_secs(5)) => {
            check_health().await.ok();
        }
        _ = cancel_rx => {
            // cancelled
        }
    }
});

// To cancel:
cancel_tx.send(()).ok();
```

**Rust — interval:**
```rust
let mut interval = tokio::time::interval(Duration::from_secs(1));
interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

loop {
    interval.tick().await;
    tick().await?;
}
```

---

## 6. Graceful Shutdown Pattern

**Node.js:**
```typescript
process.on("SIGTERM", async () => {
  await server.close();
  await db.end();
  process.exit(0);
});
```

**Rust:**
```rust
use tokio::signal;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

    // Spawn all long-lived tasks, passing shutdown_rx
    let agent_handle = tokio::spawn(
        run_agent(shutdown_tx.subscribe())
    );
    let cron_handle = tokio::spawn(
        run_cron(shutdown_tx.subscribe())
    );

    // Wait for SIGTERM or SIGINT
    signal::ctrl_c().await?;
    tracing::info!("shutdown signal — stopping");
    shutdown_tx.send(()).ok();

    // Await all tasks with timeout
    tokio::time::timeout(
        Duration::from_secs(15),
        futures::future::join_all([agent_handle, cron_handle]),
    ).await.ok();

    Ok(())
}
```

---

## 7. Middleware Pattern → Tower

OpenClaw uses Express middleware. Talon uses `tower::Layer` for the HTTP gateway:

```rust
use tower::ServiceBuilder;
use tower_http::{trace::TraceLayer, timeout::TimeoutLayer};

let app = axum::Router::new()
    .route("/message", post(handle_message))
    .layer(
        ServiceBuilder::new()
            .layer(TraceLayer::new_for_http())
            .layer(TimeoutLayer::new(Duration::from_secs(30)))
            .layer(axum::middleware::from_fn(rate_limit_middleware))
    );
```

---

## 8. Key Differences Summary

| Node.js Pattern | Tokio Equivalent | Notes |
|----------------|-----------------|-------|
| `async/await` | `async/await` + `?` | Identical surface, different execution |
| `Promise` | `Future` | Lazy in Rust — must be polled |
| `Promise.all` | `join_all` | All run concurrently |
| `Promise.race` | `select!` macro | First to complete wins |
| `EventEmitter` | `broadcast::channel` | Typed, backpressured |
| `setTimeout` | `time::sleep` | `await`-based |
| `setInterval` | `time::interval` | Returns `Interval` struct |
| `Readable` stream | `Stream` trait | `StreamExt` for combinators |
| `process.env` | `std::env::var` | No implicit access |
| `require()` / `import` | `use` | Compile-time, not runtime |
---

## Related Documents

### Depends On
- [TypeScript Pain Points](../01_Analysis/07_TypeScript_Pain_Points.md)

### See Also
- [Tokio Runtime Design](../06_Concurrency/49_Tokio_Runtime_Design.md)
- [Async Tool Execution](../06_Concurrency/50_Async_Tool_Execution.md)
- [Channel Patterns](../06_Concurrency/51_Channel_Patterns.md)

