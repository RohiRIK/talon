# Channel Patterns in Talon

> **Status:** ✅ Complete
> **Category:** Concurrency

---

## 1. Channel Type Decision Tree

```
Is there ONE sender and ONE receiver?
  └─ Yes → oneshot (one-time response)
       └─ Need many messages? → mpsc

Is there ONE sender and MANY receivers?
  └─ Yes → broadcast

Is there ONE receiver and MANY senders?
  └─ Yes → mpsc

Do you need shared mutable state across tasks?
  └─ Consider: Arc<Mutex<T>> or actor pattern
```

---

## 2. oneshot — Request/Response

Used for approval flow and any fire-once response:

```rust
// Caller side
let (tx, rx) = tokio::sync::oneshot::channel::<ApprovalDecision>();
approval_queue.send(ApprovalRequest { tx, /* ... */ }).await?;
let decision = rx.await?;  // blocks until user responds

// Handler side (in UI task)
while let Some(req) = approval_queue.recv().await {
    let decision = render_and_wait_for_user(&req).await;
    let _ = req.tx.send(decision);  // fires response
}
```

Pattern: never hold the `tx` longer than needed — drop it to signal the receiver.

---

## 3. mpsc — Work Queues

The DB actor pattern — all SQLite operations serialized through one mpsc queue:

```rust
pub struct DbActor {
    conn: rusqlite::Connection,
    rx: mpsc::Receiver<DbWork>,
}

impl DbActor {
    pub async fn run(mut self) {
        while let Some(work) = self.rx.recv().await {
            work.execute(&self.conn);
        }
        tracing::info!("DB actor shut down");
    }
}

// Client handle
pub struct DbHandle(mpsc::Sender<DbWork>);

impl DbHandle {
    pub async fn query<T: Send + 'static>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> T + Send + 'static,
    ) -> Result<T, DbError> {
        let (tx, rx) = oneshot::channel();
        self.0.send(DbWork::new(move |conn| {
            let _ = tx.send(f(conn));
        })).await.map_err(|_| DbError::ActorDead)?;
        rx.await.map_err(|_| DbError::ActorDead)
    }
}
```

---

## 4. broadcast — Event Bus

Agent events fan out to all connected gateways:

```rust
// Startup: create once
let (event_tx, _) = tokio::sync::broadcast::channel::<AgentEvent>(256);

// Each gateway subscribes
let mut telegram_rx = event_tx.subscribe();
let mut discord_rx  = event_tx.subscribe();

// Agent loop emits events
event_tx.send(AgentEvent::StreamDelta {
    session_id,
    delta: Delta::Text("Hello".into()),
}).ok();  // ok() because it's fire-and-forget; lag is acceptable

// Gateway task receives
tokio::spawn(async move {
    loop {
        match telegram_rx.recv().await {
            Ok(event) => handle_event(event).await,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("Telegram gateway lagged {n} events — some drops");
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
});
```

**Lag handling:** if a gateway is slow (e.g. Telegram rate limit), it lags.
Talon tolerates this — streaming deltas to a chat are best-effort.
Approval requests use `oneshot`, not `broadcast`, and are never dropped.

---

## 5. watch — Shared State Observation

Used for config hot-reload and agent status:

```rust
// Initial value
let (status_tx, status_rx) = tokio::sync::watch::channel(AgentStatus::Idle);

// Agent loop updates
status_tx.send(AgentStatus::Running { iteration: 1 }).ok();

// UI polls
let mut status_rx = status_rx.clone();
loop {
    status_rx.changed().await?;
    let status = status_rx.borrow().clone();
    update_ui(&status);
}
```

`watch` is perfect for "latest value wins" semantics — config, status, metrics.
Unlike `broadcast`, receivers always get the latest value even if they were slow.

---

## 6. Semaphore — Concurrency Limits

Limit concurrent LLM calls (cost control):

```rust
pub struct RateLimitedLlm {
    inner: Arc<dyn LlmProvider>,
    semaphore: Arc<Semaphore>,
}

impl RateLimitedLlm {
    pub fn new(provider: Arc<dyn LlmProvider>, max_concurrent: usize) -> Self {
        Self {
            inner: provider,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
        }
    }
}

#[async_trait]
impl LlmProvider for RateLimitedLlm {
    async fn complete(
        &self,
        req: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<Delta, LlmError>>, LlmError> {
        let _permit = self.semaphore.acquire().await
            .map_err(|_| LlmError::SemaphoreClosed)?;
        // Permit held until stream is consumed — OR use try_acquire for non-blocking
        self.inner.complete(req).await
        // _permit dropped when stream completes
    }
}
```

---

## 7. Mutex vs RwLock

| Use | Lock Type | Reason |
|-----|-----------|--------|
| `ToolRegistry` | `RwLock` | Reads dominate; writes only on skill reload |
| `ProcessStore` | `RwLock` | Many reads (poll), rare writes (spawn/kill) |
| `ApprovalMembrane.pending` | `DashMap` | Lock-free concurrent map |
| `BrowserPool` | `Mutex` | Pool mutations need exclusive access |
| `Config` | `RwLock` + `Arc` | Hot reload needs write lock briefly |

```rust
// Prefer tokio's async-aware versions — never block executor with std::sync::Mutex
use tokio::sync::{Mutex, RwLock};

// Exception: std::sync::Mutex is fine when:
// - Lock held only for synchronous, non-awaiting code
// - Very short critical section
// - Inside spawn_blocking
```

---

## 8. Channel Lifecycle & Shutdown

All channels participate in the graceful shutdown sequence:

```rust
// Shutdown coordinator
let (shutdown_tx, _) = broadcast::channel::<()>(1);

// Each task receives a subscription
let mut shutdown_rx = shutdown_tx.subscribe();

// Tasks check for shutdown via select!
tokio::select! {
    biased;
    _ = shutdown_rx.recv() => {
        // flush buffers, close connections
        return;
    }
    work = work_rx.recv() => { /* ... */ }
}

// Main: signal all tasks
shutdown_tx.send(()).ok();

// Wait for all tasks with timeout
tokio::time::timeout(Duration::from_secs(15), async {
    while let Some(res) = task_set.join_next().await {
        if let Err(e) = res { tracing::error!("Task exit error: {e}"); }
    }
}).await.ok();
```
---

## Related Documents

### Depends On
- [Tokio Runtime Design](49_Tokio_Runtime_Design.md)

### See Also
- [Async Tool Execution](50_Async_Tool_Execution.md)
- [Stream Processing](52_Stream_Processing.md)
- [Parallel Subagent Spawning](51a_Parallel_Subagent_Spawning.md)

