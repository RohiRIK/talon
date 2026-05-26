# Tokio Runtime Configuration

> **Status:** ✅ Complete
> **Category:** Concurrency

---

## 1. Runtime Selection

Talon uses a **multi-threaded Tokio runtime** for the main binary.
Sub-processes (sandbox containers) are managed as external processes,
not as async tasks.

```rust
// src/main.rs
#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> anyhow::Result<()> {
    // startup sequence
}
```

For tests, use `#[tokio::test]` which defaults to current-thread runtime
(no parallelism — deterministic test execution):

```rust
#[tokio::test]
async fn test_something() { ... }
```

---

## 2. Worker Thread Sizing

| Deployment | `worker_threads` | Rationale |
|------------|-----------------|-----------|
| Raspberry Pi / ARM | 2 | Limited cores |
| Homelab server (8-core) | 4 | I/O-bound; more threads = more context switching overhead |
| Production (32-core) | 8 | Safe upper bound for I/O-heavy agent workloads |
| Auto | omit attribute | `tokio::main` defaults to `num_cpus` |

For Talon's workload (mostly I/O-bound: LLM HTTP, SQLite, filesystem),
4 threads is the sweet spot on typical homelab hardware.

```toml
# config.toml
[runtime]
worker_threads = 4          # 0 = auto-detect
blocking_threads = 16       # tokio::task::spawn_blocking pool
```

```rust
fn build_runtime(config: &RuntimeConfig) -> tokio::runtime::Runtime {
    let mut builder = tokio::runtime::Builder::new_multi_thread();

    if config.worker_threads > 0 {
        builder.worker_threads(config.worker_threads);
    }

    builder
        .max_blocking_threads(config.blocking_threads)
        .enable_all()
        .thread_name("talon-worker")
        .on_thread_start(|| {
            tracing::debug!("Tokio worker thread started");
        })
        .build()
        .expect("Failed to build Tokio runtime")
}
```

---

## 3. Blocking Operations

SQLite and filesystem ops on slow paths use `spawn_blocking`
to avoid blocking the async executor:

```rust
// DO NOT do this — blocks executor thread:
let rows = rusqlite::Connection::open(path)?
    .execute("SELECT ...", [])?;

// DO this — moves blocking work off executor:
let rows = tokio::task::spawn_blocking(move || {
    let conn = rusqlite::Connection::open(path)?;
    conn.execute("SELECT ...", [])
}).await??;
```

Talon wraps all SQLite calls in a `DbHandle` that transparently
routes through `spawn_blocking`:

```rust
pub struct DbHandle {
    // Arc<Mutex<Connection>> inside spawn_blocking closures
    sender: mpsc::Sender<DbRequest>,
}

impl DbHandle {
    /// Single-threaded SQLite actor pattern
    /// All DB ops serialized through one thread — avoids SQLITE_BUSY
    pub async fn execute<F, T>(&self, f: F) -> Result<T, DbError>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<T, rusqlite::Error> + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.sender.send(DbRequest { f: Box::new(|conn| {
            let result = f(conn);
            let _ = tx.send(result);
        })}).await.map_err(|_| DbError::ActorDead)?;
        rx.await.map_err(|_| DbError::ActorDead)?
            .map_err(DbError::Sqlite)
    }
}
```

---

## 4. Task Hierarchy

```
tokio::main
    │
    ├── agent_loop_task (per active session)
    │       ├── llm_stream_task (per LLM call)
    │       └── tool_tasks (concurrent, JoinSet)
    │
    ├── cron_scheduler_task
    │       └── cron_agent_task (per job fire, ephemeral)
    │
    ├── gateway_tasks (one per platform)
    │       ├── telegram_recv_task
    │       ├── discord_recv_task
    │       └── http_server_task (axum)
    │
    ├── db_actor_task (serializes SQLite ops)
    ├── skill_watcher_task (notify file watcher)
    └── shutdown_listener_task
```

---

## 5. Task Spawning Conventions

```rust
// Named tasks for better panic messages and tracing
let handle = tokio::task::Builder::new()
    .name("agent-loop")
    .spawn(run_agent_loop(ctx))?;

// Detached fire-and-forget (gateway notifications)
tokio::spawn(async move {
    if let Err(e) = gateway.send(event).await {
        tracing::warn!(error = %e, "Gateway delivery failed");
    }
});

// Supervised — restart on panic
async fn supervised_task<F, Fut>(name: &'static str, factory: F)
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    loop {
        let handle = tokio::task::Builder::new()
            .name(name)
            .spawn(factory())
            .expect("spawn failed");

        match handle.await {
            Ok(()) => {
                tracing::info!(task = name, "Task exited cleanly");
                break;
            }
            Err(e) if e.is_panic() => {
                tracing::error!(task = name, "Task panicked — restarting in 1s");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => {
                tracing::error!(task = name, error = %e, "Task cancelled");
                break;
            }
        }
    }
}
```

---

## 6. select! Patterns

```rust
// Racing shutdown against work
async fn run_agent_loop(
    ctx: AgentContext,
    mut shutdown: broadcast::Receiver<()>,
) {
    loop {
        tokio::select! {
            biased;  // check shutdown first, always

            _ = shutdown.recv() => {
                tracing::info!("Agent loop shutting down");
                break;
            }

            msg = ctx.inbox.recv() => {
                match msg {
                    Some(m) => handle_message(m, &ctx).await,
                    None => break,  // inbox closed
                }
            }
        }
    }
}

// Timeout on tool execution
let result = tokio::select! {
    r = tool.execute(args, &ctx) => r,
    _ = tokio::time::sleep(tool_timeout) => {
        Err(ToolError::Timeout(tool_timeout))
    }
};
```

---

## 7. Backpressure Strategy

| Channel | Buffer Size | Drop Policy |
|---------|-------------|-------------|
| `agent_inbox` | 32 | Block sender ([backpressure](53_Resource_Limits_And_Backpressure.md)) |
| `event_bus` (broadcast) | 256 | Lag error — slow receivers drop |
| `approval_requests` | 8 | Block — must not drop |
| `db_requests` | 128 | Block sender |
| `cron_fire` | 16 | Skip if full (miss rather than delay) |

```rust
// Bounded channel — exerts backpressure on senders
let (tx, rx) = mpsc::channel::<AgentMessage>(32);

// Unbounded channel — use sparingly, only for trusted internal paths
let (tx, rx) = mpsc::unbounded_channel::<InternalEvent>();
```

Never use unbounded channels on paths that receive external input.
---

## Related Documents

### Depends On
- [Tokio Runtime Design](49_Tokio_Runtime_Design.md)

### See Also
- [Resource Limits & Backpressure](53_Resource_Limits_And_Backpressure.md)

