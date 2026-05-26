# FTS5 Full-Text Search — Deep Dive

> **Status:** ✅ Complete
> **Category:** Memory System
> **Last corrected:** dogfood pass 3

---

## 1. Why FTS5?

[FTS5 is SQLite's built-in full-text search engine. It ships with SQLite](55_SQLite_FTS5_In_Rust.md) —
zero extra dependencies, zero model downloads, instant startup.

| Feature | FTS5 | Alternative ([fastembed](59_Embedding_Retrieval.md)-rs) |
|---------|------|---------------------------|
| Startup time | ~0ms | ~2–5s (model load) |
| Disk overhead | ~30% of content size | +500MB model file |
| Query type | BM25 keyword | Semantic / cosine |
| Rust dependency | `rusqlite` flag | `fastembed` crate + ort |
| Accuracy on code | ✅ Excellent | ⚠️ Variable |
| Accuracy on prose | ✅ Good | ✅ Better |

Talon uses FTS5 as the default. `fastembed-rs` is an optional feature flag.

---

## 2. Schema & Virtual Table

```sql
-- Main content tables (source of truth)
CREATE TABLE messages (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT NOT NULL,
    role        TEXT NOT NULL,
    content     TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE TABLE memory_entries (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    key         TEXT NOT NULL UNIQUE,
    value       TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- FTS5 virtual tables (search indexes)
CREATE VIRTUAL TABLE fts_messages USING fts5(
    content,
    session_id UNINDEXED,   -- stored but not indexed
    role UNINDEXED,
    content=messages,        -- content table (external content mode)
    content_rowid=id,
    tokenize='porter unicode61'  -- porter stemming + unicode support
);

CREATE VIRTUAL TABLE fts_memory USING fts5(
    key,
    value,
    content=memory_entries,
    content_rowid=id,
    tokenize='porter unicode61'
);
```

**External content mode:** FTS5 stores no content itself — it reads from
`messages.content` on query. This avoids content duplication.
The trade-off: deletes/updates need manual FTS sync (see triggers below).

---

## 3. Sync Triggers

Keep FTS index in sync with the content tables automatically:

```sql
-- Messages: insert
CREATE TRIGGER fts_messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO fts_messages(rowid, content, session_id, role)
    VALUES (new.id, new.content, new.session_id, new.role);
END;

-- Messages: delete
CREATE TRIGGER fts_messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO fts_messages(fts_messages, rowid, content, session_id, role)
    VALUES ('delete', old.id, old.content, old.session_id, old.role);
END;

-- Messages: update
CREATE TRIGGER fts_messages_au AFTER UPDATE ON messages BEGIN
    INSERT INTO fts_messages(fts_messages, rowid, content, session_id, role)
    VALUES ('delete', old.id, old.content, old.session_id, old.role);
    INSERT INTO fts_messages(rowid, content, session_id, role)
    VALUES (new.id, new.content, new.session_id, new.role);
END;

-- Same pattern for fts_memory
CREATE TRIGGER fts_memory_ai AFTER INSERT ON memory_entries BEGIN
    INSERT INTO fts_memory(rowid, key, value)
    VALUES (new.id, new.key, new.value);
END;
```

---

## 4. Search Queries

### Basic Search

```rust
pub async fn search_messages(
    &self,
    query: &str,
    limit: usize,
) -> Result<Vec<MessageSearchHit>, MemoryError> {
    let query = query.to_string();
    self.db.query(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT
                m.id,
                m.session_id,
                m.role,
                snippet(fts_messages, 0, '<mark>', '</mark>', '…', 32) AS snippet,
                m.created_at,
                bm25(fts_messages) AS rank
             FROM fts_messages
             JOIN messages m ON fts_messages.rowid = m.id
             WHERE fts_messages MATCH ?1
             ORDER BY rank
             LIMIT ?2"
        )?;

        stmt.query_map(params![query, limit as i64], |row| {
            Ok(MessageSearchHit {
                message_id: row.get(0)?,
                session_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap(),
                role: row.get(2)?,
                snippet: row.get(3)?,
                created_at: row.get(4)?,
                rank: row.get(5)?,
            })
        })?.collect::<rusqlite::Result<_>>()
    }).await?
}
```

### FTS5 Query Syntax

Talon passes queries through to FTS5 directly — users get full power:

```
"exact phrase"          → exact match
token1 token2           → AND (both required)
token1 OR token2        → OR
token1 NOT token2       → exclude
deploy*                 → prefix wildcard
NEAR(token1 token2, 5)  → within 5 tokens of each other
```

Sanitize user input to prevent injection:

```rust
fn sanitize_fts_query(raw: &str) -> String {
    // Escape double-quotes within quoted phrases
    // Reject control characters
    raw.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .replace('"', "\"\"")  // escape embedded quotes
}
```

---

## 5. Session Search Tool Implementation

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SessionSearchParams {
    /// FTS5 query string (optional — omit to browse recent sessions)
    pub query: Option<String>,
    /// Scroll into a specific session
    pub session_id: Option<String>,
    /// Anchor message ID for scrolling
    pub around_message_id: Option<i64>,
    /// Window size for scroll mode (default: 5)
    #[serde(default = "default_window")]
    pub window: usize,
    /// Max sessions (discovery mode, default: 3)
    #[serde(default = "default_session_limit")]
    pub limit: usize,
    pub sort: Option<SearchSort>,
    pub role_filter: Option<String>,
}

#[async_trait]
impl Tool for SessionSearchTool {
    fn name(&self) -> &str { "session_search" }
    fn risk_level(&self) -> ToolRisk { ToolRisk::ReadOnly }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let p: SessionSearchParams = serde_json::from_value(args)?;

        let result = match (&p.query, &p.session_id, &p.around_message_id) {
            // BROWSE shape — no args
            (None, None, None) => {
                self.browse_recent(p.limit).await?
            }
            // SCROLL shape — session_id + anchor
            (_, Some(sid), Some(anchor)) => {
                self.scroll_session(sid, *anchor, p.window, &p.role_filter).await?
            }
            // DISCOVERY shape — query
            (Some(q), _, _) => {
                self.search(q, p.limit, p.sort.as_ref(), &p.role_filter).await?
            }
            _ => return Err(ToolError::InvalidParams(
                "Provide 'query' for discovery, 'session_id'+'around_message_id' for scroll, or nothing for browse".into()
            )),
        };

        Ok(ToolResult::text(result))
    }
}
```

---

## 6. Bookend Context for Discovery Results

Discovery results include start + end context of each matching session —
so the agent can assess relevance without fetching the full transcript:

```rust
async fn search(
    &self,
    query: &str,
    limit: usize,
    sort: Option<&SearchSort>,
    role_filter: &Option<String>,
) -> Result<String, MemoryError> {
    let hits = self.store.search_messages(query, limit * 5).await?;

    // Deduplicate by session, keep best-ranking hit per session
    let mut by_session: IndexMap<Uuid, MessageSearchHit> = IndexMap::new();
    for hit in hits {
        by_session.entry(hit.session_id)
            .and_modify(|e| { if hit.rank < e.rank { *e = hit.clone(); } })
            .or_insert(hit);
    }

    let mut results = vec![];
    for (session_id, hit) in by_session.into_iter().take(limit) {
        let session = self.store.get_session(session_id).await?;

        // Bookend: first 3 user+assistant messages
        let start = self.store.load_first_messages(session_id, 3).await?;
        // Bookend: last 3 user+assistant messages
        let end   = self.store.load_last_messages(session_id, 3).await?;
        // Window: ±5 around hit
        let window = self.store.load_window(session_id, hit.message_id, 5).await?;

        results.push(format!(
            "### Session: {} ({})\n**Match:** {}\n\n**Opening:**\n{}\n\n**Around hit:**\n{}\n\n**Closing:**\n{}",
            session.title.as_deref().unwrap_or("Untitled"),
            session.created_at,
            hit.snippet,
            format_messages(&start),
            format_messages(&window),
            format_messages(&end),
        ));
    }

    Ok(results.join("\n\n---\n\n"))
}
```

---

## 7. FTS5 Maintenance

```sql
-- Rebuild index (after bulk imports or corruption)
INSERT INTO fts_messages(fts_messages) VALUES ('rebuild');

-- Optimize (merge segments for faster queries — run weekly via cron)
INSERT INTO fts_messages(fts_messages) VALUES ('optimize');

-- Check integrity
INSERT INTO fts_messages(fts_messages) VALUES ('integrity-check');
```

```rust
// Scheduled maintenance (weekly cron job)
pub async fn optimize_fts(&self) -> Result<(), MemoryError> {
    self.db.query(|conn| {
        conn.execute("INSERT INTO fts_messages(fts_messages) VALUES ('optimize')", [])?;
        conn.execute("INSERT INTO fts_memory(fts_memory) VALUES ('optimize')", [])
    }).await??;
    tracing::info!("FTS5 optimize complete");
    Ok(())
}
```
---

## Related Documents

### Depends On
- [SQLite & FTS5 in Rust](55_SQLite_FTS5_In_Rust.md)

### See Also
- [Embedding Retrieval](59_Embedding_Retrieval.md)
- [Memory System](../04_Core_Features/35_Memory_System_SQLite_FTS5.md)
- [Cross-Session Context](56a_Cross_Session_Context.md)

