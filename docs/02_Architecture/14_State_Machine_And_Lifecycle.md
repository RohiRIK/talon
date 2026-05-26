# State Machine & Agent Lifecycle

> **Status:** ✅ Complete
> **Category:** Architecture

---

## 1. Agent States

```
                    ┌─────────────┐
                    │   CREATED   │ ← new session, config loaded
                    └──────┬──────┘
                           │ first message
                    ┌──────▼──────┐
              ┌────►│   RUNNING   │◄─────────────────┐
              │     └──────┬──────┘                   │
              │            │ tool calls pending         │
              │     ┌──────▼──────┐                   │
              │     │  EXECUTING  │ tools in flight    │
              │     └──────┬──────┘                   │
              │            │ all tools done            │
              │            └──────────────────────────►┘
              │
              │    ┌──────────────┐
              │    │  WAITING_    │ approval prompt sent,
              └────│  APPROVAL    │ awaiting user response
                   └──────┬───────┘
                          │ approved / denied
                          └──────────────────► RUNNING
                   ┌──────────────┐
                   │  SUSPENDED   │ ← /pause command, cron job paused
                   └──────┬───────┘
                          │ resume
                          └──────────────────► RUNNING
                   ┌──────────────┐
                   │  COMPLETED   │ ← natural end, max_iter reached
                   └──────────────┘
                   ┌──────────────┐
                   │   FAILED     │ ← unrecoverable error
                   └──────────────┘
```

---

## 2. State Enum

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    Created,
    Running {
        turn: u32,
        started_at: DateTime<Utc>,
    },
    Executing {
        turn: u32,
        pending_calls: Vec<String>,  // call IDs
    },
    WaitingApproval {
        turn: u32,
        call_id: String,
        tool_name: String,
        prompt: String,
    },
    Suspended {
        reason: String,
        suspended_at: DateTime<Utc>,
    },
    Completed {
        turns: u32,
        completed_at: DateTime<Utc>,
    },
    Failed {
        error: String,
        turn: u32,
        failed_at: DateTime<Utc>,
    },
}

impl AgentState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed { .. } | Self::Failed { .. })
    }

    pub fn can_receive_message(&self) -> bool {
        matches!(self, Self::Created | Self::Running { .. } | Self::Suspended { .. })
    }
}
```

---

## 3. Session Lifecycle

```rust
pub struct AgentSession {
    pub id: Uuid,
    pub state: AgentState,
    pub config: Arc<AgentConfig>,
    pub history: Vec<Message>,
    pub created_at: DateTime<Utc>,
    pub last_active: DateTime<Utc>,
    pub source: SessionSource,   // Cli, Telegram, Discord, Cron, Http
    pub profile: String,
}

#[derive(Debug, Clone)]
pub enum SessionSource {
    Cli,
    Telegram { chat_id: i64 },
    Discord { channel_id: u64 },
    Cron { job_id: Uuid },
    Http { remote_addr: String },
}

impl AgentSession {
    pub fn transition(&mut self, new_state: AgentState) -> Result<(), StateError> {
        use AgentState::*;
        let valid = match (&self.state, &new_state) {
            (Created, Running { .. }) => true,
            (Running { .. }, Executing { .. }) => true,
            (Running { .. }, WaitingApproval { .. }) => true,
            (Running { .. }, Completed { .. }) => true,
            (Running { .. }, Failed { .. }) => true,
            (Executing { .. }, Running { .. }) => true,
            (Executing { .. }, Failed { .. }) => true,
            (WaitingApproval { .. }, Running { .. }) => true,
            (WaitingApproval { .. }, Failed { .. }) => true,
            (Suspended { .. }, Running { .. }) => true,
            _ => false,
        };

        if !valid {
            return Err(StateError::InvalidTransition {
                from: format!("{:?}", self.state),
                to: format!("{:?}", new_state),
            });
        }

        self.state = new_state;
        self.last_active = Utc::now();
        Ok(())
    }
}
```

---

## 4. Session Manager

```rust
pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<Uuid, Arc<Mutex<AgentSession>>>>>,
    memory: Arc<MemoryStore>,
    ttl: Duration,
}

impl SessionManager {
    /// Create or retrieve a session for a given source
    pub async fn get_or_create(
        &self,
        source: SessionSource,
        config: Arc<AgentConfig>,
    ) -> Arc<Mutex<AgentSession>> {
        // For Telegram/Discord: one persistent session per chat_id
        // For CLI: one session per process invocation
        // For Cron: always new ephemeral session
        let key = session_key(&source);
        let sessions = self.sessions.read().await;

        if let Some(session) = sessions.get(&key) {
            return session.clone();
        }
        drop(sessions);

        let session = AgentSession {
            id: key,
            state: AgentState::Created,
            config,
            history: vec![],
            created_at: Utc::now(),
            last_active: Utc::now(),
            source,
            profile: "default".into(),
        };

        let arc = Arc::new(Mutex::new(session));
        self.sessions.write().await.insert(key, arc.clone());
        arc
    }

    /// Reap sessions idle longer than TTL
    pub async fn gc(&self) {
        let cutoff = Utc::now() - self.ttl;
        let mut sessions = self.sessions.write().await;
        sessions.retain(|_, s| {
            let s = s.blocking_lock();
            s.last_active > cutoff || !s.state.is_terminal()
        });
    }
}
```

---

## 5. Turn Lifecycle Events

```rust
pub enum TurnEvent {
    Started { session_id: Uuid, turn: u32 },
    ContextBuilt { tokens: u32, model: String },
    LlmStreamStarted { model: String },
    LlmDelta(Delta),
    LlmStreamDone { tokens_in: u32, tokens_out: u32, latency_ms: u64 },
    ToolCallQueued { call_id: String, tool: String },
    ToolCallApprovalRequired { call_id: String, tool: String, risk: ToolRisk },
    ToolCallApproved { call_id: String },
    ToolCallDenied { call_id: String },
    ToolCallStarted { call_id: String },
    ToolCallDone { call_id: String, latency_ms: u64 },
    ToolCallFailed { call_id: String, error: String },
    MemoryUpdated,
    TurnDone { turn: u32, total_tokens: u32 },
    LimitReached { reason: LimitReason },
}

pub enum LimitReason {
    MaxIterations(u32),
    TokenBudget,
    WallClock(Duration),
    UserStop,
}
```

All `TurnEvent`s are broadcast on the session's event channel. Gateway adapters subscribe and forward relevant events to users in real time.

---

## 6. Graceful Shutdown

```rust
pub async fn run_with_shutdown(
    agent: Arc<Agent>,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    loop {
        tokio::select! {
            result = agent.run_loop() => {
                if let Err(e) = result {
                    tracing::error!(error=%e, "agent loop error");
                }
                break;
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("shutdown signal received — draining");
                // Allow in-flight tool calls to complete (max 10s)
                tokio::time::timeout(
                    Duration::from_secs(10),
                    agent.drain(),
                ).await.ok();
                break;
            }
        }
    }
}
```

On `SIGTERM`/`SIGINT`, Talon completes in-flight tool calls, flushes the message buffer to SQLite, then exits cleanly.
---

## Related Documents

### Depends On
- [Core Agent Loop Design](13_Core_Agent_Loop_Design.md)

### Used By
- [Cron Scheduler](../04_Core_Features/33_Cron_Scheduler.md)

### See Also
- [Session Management](../07_Memory_System/56_Session_Management.md)
- [Profile Isolation](../04_Core_Features/40_Profile_Isolation.md)
- [Gateway Architecture](18_Gateway_MultiChannel_Architecture.md)

