# Cross-Session Context

> **Status:** ✅ Complete
> **Category:** Memory System

---

## 1. The Problem

By default, each LLM call is stateless. Talon must reconstruct context
across sessions to be useful across days and weeks of use.

There are three kinds of cross-session context:

| Kind | Volatility | Storage | Example |
|------|-----------|---------|---------|
| Permanent facts | Never changes | MEMORY.md, USER.md | User's name, coding preferences |
| Session history | Grows over time | SQLite `sessions` table | What we discussed yesterday |
| Working notes | Task-scoped | MEMORY.md (appended) | "Still working on auth PR" |

---

## 2. Memory Files

Talon maintains two flat markdown files per profile:

```
~/.talon/profiles/<name>/memories/
├── MEMORY.md    # Agent's notes about the environment, preferences, lessons
└── USER.md      # Facts about the user (name, role, timezone, preferences)
```

These are injected at Layer 2 and Layer 3 of the system prompt on every call.

### Why flat files, not just SQLite?
- Human-readable and hand-editable (user can correct mistakes)
- No query needed — always injected whole
- Git-trackable (optional)
- Small enough to fit in context (<2KB typical)

When they grow too large, Talon prunes via:
```
"Your MEMORY.md is 3,400 chars (over the 2,200 limit). Consolidate it."
```

---

## 3. Session Search

When the user references a past task, Talon searches the session transcript
DB rather than asking the user to repeat themselves.

```rust
// talon-memory/src/session_search.rs

pub struct SessionSearch {
    conn: Arc<Mutex<Connection>>,
}

impl SessionSearch {
    pub async fn search(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SessionSearchResult>, MemoryError> {
        tokio::task::spawn_blocking({
            let conn = self.conn.clone();
            let query = query.to_string();
            move || {
                let conn = conn.lock().unwrap();
                // FTS5 query
                let mut stmt = conn.prepare(
                    r#"
                    SELECT
                        m.session_id,
                        s.title,
                        s.started_at,
                        snippet(fts_messages, 0, '<b>', '</b>', '…', 32) as snippet,
                        m.id as match_id
                    FROM fts_messages f
                    JOIN messages m ON m.id = f.rowid
                    JOIN sessions s ON s.id = m.session_id
                    WHERE fts_messages MATCH ?1
                    ORDER BY rank
                    LIMIT ?2
                    "#
                )?;

                let results = stmt.query_map(params![query, limit], |row| {
                    Ok(SessionSearchResult {
                        session_id: row.get(0)?,
                        title: row.get(1)?,
                        started_at: row.get(2)?,
                        snippet: row.get(3)?,
                        match_message_id: row.get(4)?,
                    })
                })?.collect::<rusqlite::Result<Vec<_>>>()?;

                Ok(results)
            }
        })
        .await
        .map_err(|e| MemoryError::JoinError(e.to_string()))?
    }

    /// Get ±window messages around a specific message (scroll shape)
    pub async fn scroll(
        &self,
        session_id: &str,
        around_message_id: i64,
        window: u32,
    ) -> Result<Vec<Message>, MemoryError> {
        tokio::task::spawn_blocking({
            let conn = self.conn.clone();
            let session_id = session_id.to_string();
            move || {
                let conn = conn.lock().unwrap();
                let mut stmt = conn.prepare(
                    r#"
                    SELECT id, role, content, created_at
                    FROM messages
                    WHERE session_id = ?1
                      AND id BETWEEN ?2 - ?3 AND ?2 + ?3
                    ORDER BY id
                    "#
                )?;
                let messages = stmt.query_map(
                    params![session_id, around_message_id, window],
                    |row| Ok(Message {
                        id: row.get(0)?,
                        role: row.get(1)?,
                        content: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                )?.collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(messages)
            }
        })
        .await
        .map_err(|e| MemoryError::JoinError(e.to_string()))?
    }
}
```

---

## 4. Context Window Budget

Talon tracks how many tokens each context layer consumes:

```rust
pub struct ContextBudget {
    pub total_limit: u32,       // e.g., 180_000 for claude-3.7
    pub system_reserved: u32,   // ~8_000 for base system prompt
    pub memory_used: u32,
    pub history_used: u32,
    pub skill_used: u32,
}

impl ContextBudget {
    pub fn available_for_history(&self) -> u32 {
        self.total_limit
            .saturating_sub(self.system_reserved)
            .saturating_sub(self.memory_used)
            .saturating_sub(self.skill_used)
            .saturating_sub(8_000)  // reserve for output
    }
}
```

If history exceeds budget, oldest messages are pruned (keeping system
prompt + recent N messages):

```rust
pub fn trim_history(
    messages: Vec<Message>,
    token_budget: u32,
    tokenizer: &dyn Tokenizer,
) -> Vec<Message> {
    let mut kept = vec![];
    let mut tokens = 0u32;

    // Always keep system message + last user message
    // Walk backwards and keep as many as fit
    for msg in messages.iter().rev() {
        let t = tokenizer.count(&msg.content);
        if tokens + t > token_budget {
            break;
        }
        tokens += t;
        kept.push(msg.clone());
    }

    kept.reverse();
    kept
}
```

---

## 5. Memory Operations as Tools

Talon exposes memory to the LLM as callable tools:

| Tool | Description |
|------|-------------|
| `memory_add` | Add a fact to MEMORY.md |
| `memory_replace` | Replace/update an existing entry |
| `memory_remove` | Delete an entry |
| `session_search` | Search past sessions by query |

These let the LLM proactively save facts mid-conversation:

```json
{
  "tool": "memory_add",
  "parameters": {
    "target": "user",
    "content": "User prefers pnpm over npm. Uses Next.js App Router."
  }
}
```
---

## Related Documents

### Depends On
- [Session Management](56_Session_Management.md)

### See Also
- [Context & Memory Architecture](../02_Architecture/15_Context_And_Memory_Architecture.md)
- [FTS5 Search Deep Dive](58_FTS5_Search_Deep_Dive.md)

