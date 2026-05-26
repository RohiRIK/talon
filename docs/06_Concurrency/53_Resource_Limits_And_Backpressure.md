# Resource Limits & Backpressure

> **Status:** ✅ Complete
> **Category:** Concurrency

---

## 1. Why Limits Matter

Talon runs autonomously for long periods. Without limits, a misbehaving
LLM can:
- Spawn infinite tool calls
- Fill disk with subprocess output
- OOM the machine with parallel agent runs
- Trigger rate-limit bans from providers

Hard limits are safety rails, not performance optimizations.

---

## 2. Limit Hierarchy

```
Global (AppState)
├── max_concurrent_sessions: 10
├── max_subagents_total: 20
└── max_disk_write_mb_per_run: 500

Per Session (AgentRun)
├── max_iterations: 100
├── max_tool_calls_per_turn: 10
├── max_parallel_tool_calls: 5
├── max_output_bytes_per_tool: 50_000
└── timeout_per_turn: 300s

Per Tool
├── terminal: max_output = 50KB, timeout = 180s
├── web_extract: max_pages = 5, timeout = 30s/page
├── file_write: max_size = 10MB
└── code_exec: max_time = 300s, no_network = true
```

---

## 3. Iteration Limit

```rust
pub struct AgentLoopConfig {
    pub max_iterations: u32,
    pub max_tool_calls_per_turn: u32,
    pub timeout: Duration,
}

impl AgentLoop {
    pub async fn run(&mut self, input: AgentInput) -> Result<AgentOutput, AgentError> {
        let mut iterations = 0u32;
        let deadline = Instant::now() + self.config.timeout;

        loop {
            if iterations >= self.config.max_iterations {
                return Err(AgentError::MaxIterationsExceeded {
                    limit: self.config.max_iterations,
                });
            }

            if Instant::now() > deadline {
                return Err(AgentError::Timeout {
                    elapsed: self.config.timeout,
                });
            }

            iterations += 1;
            let result = self.step().await?;

            if result.is_terminal() {
                return Ok(AgentOutput { iterations, .. });
            }
        }
    }
}
```

---

## 4. Concurrent Session Semaphore

```rust
pub struct AppState {
    session_semaphore: Arc<Semaphore>,
    // ...
}

impl AppState {
    pub fn new(config: &Config) -> Self {
        Self {
            session_semaphore: Arc::new(Semaphore::new(config.max_concurrent_sessions)),
            // ...
        }
    }

    pub async fn handle_input(&self, input: AgentInput) -> Result<(), AgentError> {
        // Try to acquire a session slot (non-blocking)
        let permit = self.session_semaphore
            .try_acquire()
            .map_err(|_| AgentError::TooManyConcurrentSessions {
                max: self.config.max_concurrent_sessions,
            })?;

        // permit dropped when session completes → slot released
        tokio::spawn(async move {
            let _guard = permit;
            self.run_agent_session(input).await.ok();
        });

        Ok(())
    }
}
```

---

## 5. Tool Output Size Limiter

```rust
pub struct SizeLimitedWriter {
    inner: Vec<u8>,
    limit: usize,
    truncated: bool,
}

impl Write for SizeLimitedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let remaining = self.limit.saturating_sub(self.inner.len());
        if remaining == 0 {
            self.truncated = true;
            return Ok(buf.len());  // Pretend we wrote it (sink)
        }
        let to_write = buf.len().min(remaining);
        self.inner.extend_from_slice(&buf[..to_write]);
        if to_write < buf.len() {
            self.truncated = true;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

impl SizeLimitedWriter {
    pub fn into_string(self) -> String {
        let mut s = String::from_utf8_lossy(&self.inner).into_owned();
        if self.truncated {
            s.push_str("\n[Output truncated — exceeded 50KB limit]");
        }
        s
    }
}
```

---

## 6. Subagent Spawn Limit

```rust
pub struct SubagentTracker {
    active: Arc<AtomicU32>,
    max: u32,
}

impl SubagentTracker {
    pub fn try_spawn(&self) -> Result<SubagentGuard, AgentError> {
        let current = self.active.fetch_add(1, Ordering::Relaxed);
        if current >= self.max {
            self.active.fetch_sub(1, Ordering::Relaxed);
            return Err(AgentError::MaxSubagentsExceeded { max: self.max });
        }
        Ok(SubagentGuard { tracker: self.active.clone() })
    }
}

pub struct SubagentGuard {
    tracker: Arc<AtomicU32>,
}

impl Drop for SubagentGuard {
    fn drop(&mut self) {
        self.tracker.fetch_sub(1, Ordering::Relaxed);
    }
}
```

---

## 7. Backpressure on Channel Capacity

| Channel | Capacity | Full Behavior |
|---------|----------|---------------|
| `agent_event_tx` → TUI | 256 | `send()` awaits (TUI must drain) |
| `agent_event_tx` → Telegram | 32 | `try_send()` drops oldest |
| `tool_output_tx` → display | 64 | `send()` awaits |
| `input_rx` from gateways | 128 | Gateway blocks (applies backpressure upstream) |

Blocking is safe for TUI and logging sinks (fast consumers).
Telegram delivery uses drop-oldest semantics to avoid stalling the agent
if the Telegram API is temporarily rate-limited.

---

## 8. Rate Limit Circuit Breaker

```rust
pub struct RateLimitState {
    failures: u32,
    last_failure: Option<Instant>,
    cooldown: Duration,
}

impl RateLimitState {
    pub fn should_attempt(&self) -> bool {
        match self.last_failure {
            None => true,
            Some(t) => {
                // Exponential backoff: 2^failures seconds, max 300s
                let wait = Duration::from_secs(
                    (2u64.pow(self.failures.min(8))) * 1000 / 1000
                ).min(self.cooldown);
                t.elapsed() > wait
            }
        }
    }

    pub fn record_failure(&mut self) {
        self.failures += 1;
        self.last_failure = Some(Instant::now());
    }

    pub fn record_success(&mut self) {
        self.failures = 0;
        self.last_failure = None;
    }
}
```
---

## Related Documents

### See Also
- [Tokio Runtime Config](50a_Tokio_Runtime_Config.md)
- [Security Model](../02_Architecture/20_Security_Model.md)
- [Parallel Subagent Spawning](51a_Parallel_Subagent_Spawning.md)

