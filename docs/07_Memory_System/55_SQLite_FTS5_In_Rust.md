# SQLite + FTS5 in Rust

> **Status:** ✅ Complete
> **Category:** Memory System

---

## 1. Why FTS5 Over Embeddings (Default)

| Factor | FTS5 | [fastembed](59_Embedding_Retrieval.md)-rs |
|--------|------|--------------|
| Install | Zero (bundled) | ~300MB model download |
| Cold-start latency | <1ms | ~3s (model load) |
| Query latency | ~0.1ms | ~20ms (embedding) |
| Multi-language | ✅ unicode61 tokenizer | ✅ multilingual models |
| Exact phrase match | ✅ | ❌ |
| Semantic similarity | ❌ | ✅ |
| Disk space | +15% vs raw | +200MB |

**Decision:** FTS5 is default. Semantic retrieval is `feature = "embeddings"` opt-in.

---

## 2. rusqlite + Bundled FTS5

```toml
[dependencies]
rusqlite = { version = "0.31", features = ["bundled", "vtab", "functions"] }
```

The `bundled` feature compiles SQLite 3.x from source — no system sqlite3 dep. `vtab` enables FTS5 virtual tables.

---

## 3. Schema & Triggers

```rust
pub fn apply_migrations(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch("
        PRAGMA journal_mode=WAL;
        PRAGMA synchronous=NORMAL;
        PRAGMA foreign_keys=ON;

        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            title TEXT,
            source TEXT,
            profile TEXT DEFAULT 'default',
            created_at INTEGER NOT NULL DEFAULT (unixepoch()),
            updated_at INTEGER NOT NULL DEFAULT (unixepoch())
        );

        CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at INTEGER NOT NULL DEFAULT (unixepoch())
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS fts_messages USING fts5(
            content,
            session_id UNINDEXED,
            message_id UNINDEXED,
            tokenize = 'porter unicode61 remove_diacritics 1'
        );

        -- Keep FTS index in sync
        CREATE TRIGGER IF NOT EXISTS messages_ai
        AFTER INSERT ON messages BEGIN
            INSERT INTO fts_messages(content, session_id, message_id)
            VALUES (new.content, new.session_id, new.id);
        END;

        CREATE TRIGGER IF NOT EXISTS messages_ad
        AFTER DELETE ON messages BEGIN
            DELETE FROM fts_messages WHERE message_id = old.id;
        END;

        CREATE TRIGGER IF NOT EXISTS messages_au
        AFTER UPDATE ON messages BEGIN
            DELETE FROM fts_messages WHERE message_id = old.id;
            INSERT INTO fts_messages(content, session_id, message_id)
            VALUES (new.content, new.session_id, new.id);
        END;
    ")
}
```

---

## 4. FTS5 Search Query

```rust
pub fn search_messages(
    conn: &rusqlite::Connection,
    query: &str,
    limit: u32,
) -> rusqlite::Result<Vec<SearchHit>> {
    // FTS5 highlight() and snippet() built-ins
    let sql = "
        SELECT
            m.id,
            m.session_id,
            m.role,
            snippet(fts_messages, 0, '<b>', '</b>', '...', 20) AS snippet,
            rank
        FROM fts_messages
        JOIN messages m ON m.id = fts_messages.message_id
        WHERE fts_messages MATCH ?1
        ORDER BY rank
        LIMIT ?2
    ";

    let mut stmt = conn.prepare(sql)?;
    let hits = stmt.query_map(
        rusqlite::params![query, limit],
        |row| Ok(SearchHit {
            message_id: row.get(0)?,
            session_id: row.get::<_, String>(1)?.parse().unwrap(),
            role: row.get(2)?,
            snippet: row.get(3)?,
            rank: row.get(4)?,
        }),
    )?.collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(hits)
}
```

---

## 5. FTS5 Query Syntax Support

FTS5 supports:
- `AND` (implicit between terms): `tokio async`
- `OR`: `tokio OR async`
- `NOT`: `tokio NOT async`
- Phrase: `"tokio runtime"`
- Prefix: `deploy*`
- Column filter: `content:deploy`

Talon exposes these via the `session_search` tool with a `query` parameter — the raw FTS5 syntax is passed through.

---

## 6. Session Retrieval with Context Window

```rust
pub fn get_messages_around(
    conn: &rusqlite::Connection,
    session_id: Uuid,
    anchor_id: i64,
    window: u32,
) -> rusqlite::Result<Vec<DbMessage>> {
    let sql = "
        SELECT id, role, content, created_at
        FROM messages
        WHERE session_id = ?1
          AND id BETWEEN ?2 AND ?3
        ORDER BY id ASC
    ";

    let before = anchor_id - window as i64;
    let after  = anchor_id + window as i64;

    conn.prepare(sql)?
        .query_map(
            rusqlite::params![session_id.to_string(), before.max(0), after],
            DbMessage::from_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()
}
```

---

## 7. WAL Mode + Connection Pool

```rust
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

pub fn create_pool(db_path: &Path) -> Result<Pool<SqliteConnectionManager>, r2d2::Error> {
    let manager = SqliteConnectionManager::file(db_path)
        .with_init(|conn| {
            conn.execute_batch("
                PRAGMA journal_mode=WAL;
                PRAGMA synchronous=NORMAL;
                PRAGMA cache_size=-64000;
                PRAGMA temp_store=MEMORY;
            ")
        });
    Pool::builder().max_size(8).build(manager)
}
```

WAL allows multiple concurrent readers + one writer. Ideal for the agent loop (writer) + [cron scheduler](../04_Core_Features/33_Cron_Scheduler.md) (writer) + FTS search (reader) running concurrently.

---

## 8. spawn_blocking Pattern

rusqlite is synchronous. All DB calls in async context use `spawn_blocking`:

```rust
pub async fn insert_message(
    &self,
    session_id: Uuid,
    role: &str,
    content: String,
) -> Result<i64, MemoryError> {
    let pool = self.pool.clone();
    let role = role.to_string();
    let sid = session_id.to_string();

    tokio::task::spawn_blocking(move || {
        let conn = pool.get()?;
        conn.execute(
            "INSERT INTO messages(session_id, role, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![sid, role, content],
        )?;
        Ok::<i64, MemoryError>(conn.last_insert_rowid())
    }).await?
}
```
---

## Related Documents

### Depends On
- [Cargo Workspace Design](../02_Architecture/12_Workspace_And_Crate_Structure.md)

### Used By
- [Context & Memory Architecture](../02_Architecture/15_Context_And_Memory_Architecture.md)
- [Memory System](../04_Core_Features/35_Memory_System_SQLite_FTS5.md)

### See Also
- [FTS5 Search Deep Dive](58_FTS5_Search_Deep_Dive.md)
- [Session Management](56_Session_Management.md)
- [Embedding Retrieval](59_Embedding_Retrieval.md)

