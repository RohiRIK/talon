# Session Management

> **Status:** ✅ Complete
> **Category:** Memory System

---

## 1. Session Lifecycle

```
inbound message (any gateway)
        │
        ▼
  SessionManager.get_or_create(source, chat_id, thread_id)
        │
        ├── Existing session? ──► load messages from SQLite
        │                         update session.updated_at
        │
        └── New session? ──────► INSERT sessions row
                                  set title from first message
        │
        ▼
   AgentContext { session_id, messages, ... }
        │
        ▼
   agent_loop.run(ctx)
        │
        ▼
   Each message saved to SQLite immediately after agent turn
        │
        ▼
   Session stays "open" until:
     - 30min idle timeout
     - /new command
     - Gateway disconnect
```

---

## 2. SQLite Schema

```sql
CREATE TABLE sessions (
    id          TEXT PRIMARY KEY,   -- UUID
    title       TEXT,
    source      TEXT NOT NULL,      -- "telegram", "discord", "cli"
    profile     TEXT NOT NULL DEFAULT 'default',
    chat_id     TEXT,               -- platform-specific chat ID
    thread_id   TEXT,               -- platform-specific thread/topic ID
    created_at  INTEGER NOT NULL,   -- Unix timestamp (seconds)
    updated_at  INTEGER NOT NULL,
    metadata    TEXT                -- JSON blob for extra data
);

CREATE TABLE messages (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role        TEXT NOT NULL,      -- user | assistant | system | tool
    content     TEXT NOT NULL,      -- JSON (string or content blocks)
    tool_call_id TEXT,
    name        TEXT,
    created_at  INTEGER NOT NULL,
    token_count INTEGER             -- cached estimate
);

CREATE INDEX idx_messages_session ON messages(session_id, created_at);
CREATE INDEX idx_sessions_source  ON sessions(source, updated_at DESC);
```

---

## 3. SessionManager

```rust
pub struct SessionManager {
    db: DbHandle,
    open_sessions: DashMap<Uuid, SessionHandle>,
    idle_timeout: Duration,
}

pub struct SessionHandle {
    pub id: Uuid,
    pub last_activity: Arc<RwLock<Instant>>,
    pub inbox: mpsc::Sender<InboundMessage>,
}

impl SessionManager {
    pub async fn get_or_create(
        &self,
        source: &str,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> Result<SessionHandle, SessionError> {
        // Look up existing open session
        let key = session_key(source, chat_id, thread_id);
        if let Some(handle) = self.open_sessions.get(&key) {
            handle.touch().await;
            return Ok(handle.clone());
        }

        // Look up in DB (session from a previous run)
        let existing = self.db.query(|conn| {
            conn.query_row(
                "SELECT id FROM sessions
                 WHERE source = ?1 AND chat_id = ?2 AND thread_id IS ?3
                 ORDER BY updated_at DESC LIMIT 1",
                params![source, chat_id, thread_id],
                |row| row.get::<_, String>(0),
            ).optional()
        }).await?;

        let session_id = if let Some(id_str) = existing {
            Uuid::parse_str(&id_str)?
        } else {
            // Create new session
            let id = Uuid::new_v4();
            let now = Utc::now().timestamp();
            self.db.query(move |conn| {
                conn.execute(
                    "INSERT INTO sessions(id, source, chat_id, thread_id, profile, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, 'default', ?5, ?5)",
                    params![id.to_string(), source, chat_id, thread_id, now],
                )
            }).await??;
            id
        };

        let handle = self.spawn_session_task(session_id).await?;
        self.open_sessions.insert(session_id, handle.clone());
        Ok(handle)
    }

    /// Load recent messages respecting token budget
    pub async fn load_context(
        &self,
        session_id: Uuid,
        token_budget: u32,
    ) -> Result<Vec<Message>, SessionError> {
        let rows = self.db.query(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT role, content, tool_call_id, name, token_count
                 FROM messages WHERE session_id = ?1
                 ORDER BY created_at DESC"
            )?;
            let rows: Vec<(String, String, Option<String>, Option<String>, Option<u32>)> =
                stmt.query_map(params![session_id.to_string()], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })?
                .collect::<rusqlite::Result<_>>()?;
            Ok(rows)
        }).await??;

        // Take messages newest-first until budget exhausted, then reverse
        let mut budget_remaining = token_budget;
        let mut selected = vec![];

        for (role, content_json, tool_call_id, name, token_est) in rows {
            let tokens = token_est.unwrap_or_else(|| estimate_tokens(&content_json));
            if budget_remaining < tokens { break; }
            budget_remaining -= tokens;

            let content: MessageContent = serde_json::from_str(&content_json)
                .unwrap_or(MessageContent::Text(content_json));

            selected.push(Message {
                role: role.parse()?,
                content,
                tool_call_id,
                name,
            });
        }

        selected.reverse();  // chronological order
        Ok(selected)
    }

    /// Persist a message immediately after it's produced
    pub async fn append_message(
        &self,
        session_id: Uuid,
        msg: &Message,
    ) -> Result<i64, SessionError> {
        let content_json = serde_json::to_string(&msg.content)?;
        let token_est = estimate_tokens(&content_json);
        let now = Utc::now().timestamp();

        let id = self.db.query(move |conn| {
            conn.execute(
                "INSERT INTO messages(session_id, role, content, tool_call_id, name, created_at, token_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    session_id.to_string(),
                    msg.role.as_str(),
                    content_json,
                    msg.tool_call_id,
                    msg.name,
                    now,
                    token_est,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        }).await??;

        // Update session.updated_at
        self.db.query(move |conn| {
            conn.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                params![now, session_id.to_string()],
            )
        }).await??;

        Ok(id)
    }
}
```

---

## 4. Session Title Generation

First user message → title set async (doesn't block response):

```rust
async fn maybe_set_title(
    session_id: Uuid,
    first_message: &str,
    db: &DbHandle,
) {
    // Check if title already set
    let has_title = db.query(move |conn| {
        conn.query_row(
            "SELECT title IS NOT NULL FROM sessions WHERE id = ?1",
            params![session_id.to_string()],
            |r| r.get::<_, bool>(0),
        )
    }).await.unwrap_or(Ok(true)).unwrap_or(true);

    if has_title { return; }

    // Truncate to first 60 chars of first meaningful sentence
    let title = first_message
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(first_message)
        .chars()
        .take(60)
        .collect::<String>();

    db.query(move |conn| {
        conn.execute(
            "UPDATE sessions SET title = ?1 WHERE id = ?2 AND title IS NULL",
            params![title, session_id.to_string()],
        )
    }).await.ok();
}
```

---

## 5. Idle Session Cleanup

```rust
async fn session_gc_task(
    manager: Arc<SessionManager>,
    idle_timeout: Duration,
    mut shutdown: broadcast::Receiver<()>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));

    loop {
        tokio::select! {
            _ = shutdown.recv() => break,
            _ = interval.tick() => {
                let cutoff = Instant::now() - idle_timeout;
                manager.open_sessions.retain(|_, handle| {
                    // Keep sessions with recent activity
                    *handle.last_activity.try_read()
                        .map(|t| *t > cutoff)
                        .unwrap_or(true)  // can't read lock = keep
                });
            }
        }
    }
}
```
---

## Related Documents

### Depends On
- [SQLite & FTS5 in Rust](55_SQLite_FTS5_In_Rust.md)

### See Also
- [Cross-Session Context](56a_Cross_Session_Context.md)
- [State Machine & Lifecycle](../02_Architecture/14_State_Machine_And_Lifecycle.md)
- [Profile Isolation](../04_Core_Features/40_Profile_Isolation.md)

