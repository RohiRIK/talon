# Session & Conversation Management

> **Status:** ✅ Complete
> **Category:** Core Features

---

## 1. What is a Session?

A session is a continuous conversation thread. Multiple sessions can exist
simultaneously (one per gateway source / user / thread).

Sessions have:
- A unique ID (UUID)
- A title (auto-generated from first message)
- Message history
- An associated profile
- Optional workdir context

---

## 2. SQLite Schema

```sql
CREATE TABLE sessions (
    id          TEXT PRIMARY KEY,          -- UUID
    profile     TEXT NOT NULL,             -- profile name
    title       TEXT,                      -- first message summary
    platform    TEXT,                      -- "telegram", "cli", "http"
    chat_id     TEXT,                      -- platform-specific chat ID
    thread_id   TEXT,                      -- optional thread (Telegram topics)
    started_at  INTEGER NOT NULL,          -- Unix timestamp
    last_active INTEGER NOT NULL,
    metadata    TEXT                       -- JSON (workdir, model used, etc.)
);

CREATE TABLE messages (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role        TEXT NOT NULL,             -- "user" | "assistant" | "tool" | "tool_result"
    content     TEXT NOT NULL,
    tool_use_id TEXT,                      -- for role=tool_result
    tool_name   TEXT,                      -- for role=tool
    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    tokens      INTEGER                    -- cached token count
);

CREATE INDEX idx_messages_session ON messages(session_id, created_at);

-- FTS5 virtual table for semantic search
CREATE VIRTUAL TABLE fts_messages USING fts5(
    content,
    content=messages,
    content_rowid=id
);

-- Triggers to keep FTS in sync
CREATE TRIGGER messages_fts_insert AFTER INSERT ON messages BEGIN
    INSERT INTO fts_messages(rowid, content) VALUES (new.id, new.content);
END;
```

---

## 3. Session Manager

```rust
pub struct SessionManager {
    conn: Arc<Mutex<Connection>>,
}

impl SessionManager {
    pub async fn get_or_create(
        &self,
        platform: &str,
        chat_id: &str,
        thread_id: Option<&str>,
        profile: &str,
    ) -> Result<Session, MemoryError> {
        tokio::task::spawn_blocking({
            let conn = self.conn.clone();
            let platform = platform.to_string();
            let chat_id = chat_id.to_string();
            let thread_id = thread_id.map(str::to_string);
            let profile = profile.to_string();
            move || {
                let conn = conn.lock().unwrap();
                // Try to find an active session for this chat
                let existing: Option<Session> = conn.query_row(
                    "SELECT id, title, started_at FROM sessions
                     WHERE platform = ?1 AND chat_id = ?2 AND thread_id IS ?3
                     ORDER BY last_active DESC LIMIT 1",
                    params![platform, chat_id, thread_id],
                    |row| Ok(Session {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        started_at: row.get(2)?,
                    })
                ).optional()?;

                if let Some(s) = existing {
                    return Ok(s);
                }

                // Create new session
                let id = Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO sessions (id, profile, platform, chat_id, thread_id, started_at, last_active)
                     VALUES (?1, ?2, ?3, ?4, ?5, unixepoch(), unixepoch())",
                    params![id, profile, platform, chat_id, thread_id],
                )?;
                Ok(Session { id, title: None, started_at: SystemTime::now() })
            }
        }).await.map_err(|e| MemoryError::JoinError(e.to_string()))?
    }

    pub async fn append_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        tool_use_id: Option<&str>,
        tool_name: Option<&str>,
    ) -> Result<i64, MemoryError> {
        tokio::task::spawn_blocking({
            let conn = self.conn.clone();
            let session_id = session_id.to_string();
            let role = role.to_string();
            let content = content.to_string();
            let tool_use_id = tool_use_id.map(str::to_string);
            let tool_name = tool_name.map(str::to_string);
            move || {
                let conn = conn.lock().unwrap();
                conn.execute(
                    "INSERT INTO messages (session_id, role, content, tool_use_id, tool_name)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![session_id, role, content, tool_use_id, tool_name],
                )?;
                conn.execute(
                    "UPDATE sessions SET last_active = unixepoch() WHERE id = ?1",
                    params![session_id],
                )?;
                Ok(conn.last_insert_rowid())
            }
        }).await.map_err(|e| MemoryError::JoinError(e.to_string()))?
    }

    pub async fn load_history(
        &self,
        session_id: &str,
        limit: u32,
    ) -> Result<Vec<Message>, MemoryError> {
        tokio::task::spawn_blocking({
            let conn = self.conn.clone();
            let session_id = session_id.to_string();
            move || {
                let conn = conn.lock().unwrap();
                let mut stmt = conn.prepare(
                    "SELECT id, role, content, tool_use_id, tool_name, created_at
                     FROM messages
                     WHERE session_id = ?1
                     ORDER BY created_at DESC
                     LIMIT ?2"
                )?;
                let mut messages: Vec<Message> = stmt.query_map(params![session_id, limit], |row| {
                    Ok(Message {
                        id: row.get(0)?,
                        role: row.get(1)?,
                        content: row.get(2)?,
                        tool_use_id: row.get(3)?,
                        tool_name: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                })?.collect::<rusqlite::Result<_>>()?;
                messages.reverse();  // Oldest first for LLM context
                Ok(messages)
            }
        }).await.map_err(|e| MemoryError::JoinError(e.to_string()))?
    }
}
```

---

## 4. Session Title Generation

After the first assistant response, Talon auto-generates a session title:

```rust
pub async fn auto_title_session(
    session_manager: &SessionManager,
    llm: &dyn LlmProvider,
    session_id: &str,
    first_user_message: &str,
) -> Result<(), Error> {
    // Quick, cheap title generation — no tools, 1 turn
    let title = llm.complete_simple(
        "Generate a 3-5 word title for a conversation starting with: \"{}\"
         Reply with ONLY the title, no quotes, no punctuation at the end.",
        first_user_message,
    ).await?;

    session_manager.set_title(session_id, title.trim()).await?;
    Ok(())
}
```

---

## 5. New Session Command

Users can start a fresh session mid-conversation:

```
/new        → fresh session, same profile
/new --profile work  → fresh session with different profile
```

Talon asks if there's work-in-progress before discarding the old session:

```rust
if session_has_uncommitted_work(&current_session).await? {
    deliver("You have work in progress. Start a new session anyway? [Y/n]").await;
    // Wait for approval...
}
```
---

## Related Documents

### Depends On
- [Context & Memory Architecture](../02_Architecture/15_Context_And_Memory_Architecture.md)
- [SQLite & FTS5 in Rust](../07_Memory_System/55_SQLite_FTS5_In_Rust.md)

### See Also
- [Session Management](../07_Memory_System/56_Session_Management.md)
- [Cross-Session Context](../07_Memory_System/56a_Cross_Session_Context.md)

