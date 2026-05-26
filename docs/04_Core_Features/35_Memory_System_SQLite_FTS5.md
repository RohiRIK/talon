# Memory System: SQLite + FTS5

> **Status:** ✅ Complete
> **Category:** Core Features

---

## 1. Storage Architecture

Talon uses SQLite as its single persistence layer:

```
~/.talon/profiles/<name>/
├── db/talon.db       # SQLite database (sessions, messages, entities, FTS5)
├── memories/
│   ├── MEMORY.md      # Agent notes (injected as context)
│   └── USER.md        # User profile (injected as context)
└── skills/            # Skill markdown files
```

All persistent state flows through `talon.db`. The markdown files
(MEMORY.md, USER.md) are small enough to be injected whole into context.

---

## 2. Why SQLite?

| Requirement | SQLite | Postgres | Redis |
|-------------|--------|----------|-------|
| Zero setup | ✅ | ❌ needs server | ❌ needs server |
| Local-first | ✅ | ❌ | ❌ |
| Full-text search | ✅ FTS5 | ✅ | ❌ |
| ACID | ✅ | ✅ | Partial |
| File-based (portable) | ✅ | ❌ | ❌ |
| Backup simplicity | `cp talon.db` | `pg_dump` | `rdb` |

SQLite is the right choice for a personal AI agent with one active user.

---

## 3. Connection Management

`[rusqlite](../07_Memory_System/55_SQLite_FTS5_In_Rust.md)` is synchronous. All DB operations run in `spawn_blocking`:

```rust
pub struct Database {
    // Single write connection, WAL mode
    write_conn: Arc<Mutex<Connection>>,
    // Multiple read connections via pool
    read_pool: Arc<Pool<SqliteConnectionManager>>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, DatabaseError> {
        let conn = Connection::open(path)?;

        // Enable WAL for concurrent reads during writes
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;

        Self::run_migrations(&conn)?;
        Ok(Self {
            write_conn: Arc::new(Mutex::new(conn)),
            read_pool: Arc::new(Pool::new(
                SqliteConnectionManager::file(path)
            )?),
        })
    }

    pub async fn read<T, F>(&self, f: F) -> Result<T, DatabaseError>
    where
        F: FnOnce(&Connection) -> Result<T, rusqlite::Error> + Send + 'static,
        T: Send + 'static,
    {
        let pool = self.read_pool.clone();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            f(&conn).map_err(DatabaseError::Rusqlite)
        }).await.map_err(|e| DatabaseError::JoinError(e.to_string()))?
    }

    pub async fn write<T, F>(&self, f: F) -> Result<T, DatabaseError>
    where
        F: FnOnce(&Connection) -> Result<T, rusqlite::Error> + Send + 'static,
        T: Send + 'static,
    {
        let conn = self.write_conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            f(&conn).map_err(DatabaseError::Rusqlite)
        }).await.map_err(|e| DatabaseError::JoinError(e.to_string()))?
    }
}
```

---

## 4. FTS5 Full-Text Search

FTS5 is a SQLite extension for full-text search. Talon uses it for
session history retrieval via the `session_search` tool.

```sql
-- Virtual table mirrors messages content
CREATE VIRTUAL TABLE fts_messages USING fts5(
    content,
    content='messages',       -- external content table
    content_rowid='id'        -- maps to messages.id
);

-- Sync triggers
CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO fts_messages(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO fts_messages(fts_messages, rowid, content)
    VALUES('delete', old.id, old.content);
END;

CREATE TRIGGER messages_au AFTER UPDATE ON messages BEGIN
    INSERT INTO fts_messages(fts_messages, rowid, content)
    VALUES('delete', old.id, old.content);
    INSERT INTO fts_messages(rowid, content) VALUES (new.id, new.content);
END;
```

FTS5 supports:
- Boolean: `rust AND tokio`
- Phrases: `"async runtime"`
- Prefix: `embed*`
- Column filters: `content:error`
- Ranking: `bm25()` by default

---

## 5. Migrations

Talon uses sequential SQL migrations stored as embedded strings:

```rust
const MIGRATIONS: &[(&str, &str)] = &[
    ("001_initial", include_str!("migrations/001_initial.sql")),
    ("002_add_entities", include_str!("migrations/002_add_entities.sql")),
    ("003_fts5", include_str!("migrations/003_fts5.sql")),
];

fn run_migrations(conn: &Connection) -> Result<(), DatabaseError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version TEXT PRIMARY KEY,
             applied_at INTEGER NOT NULL DEFAULT (unixepoch())
         );"
    )?;

    for (version, sql) in MIGRATIONS {
        let already_applied: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
            params![version],
            |row| row.get(0),
        )?;

        if !already_applied {
            conn.execute_batch(sql)?;
            conn.execute(
                "INSERT INTO schema_migrations (version) VALUES (?1)",
                params![version],
            )?;
            tracing::info!("Applied migration: {}", version);
        }
    }
    Ok(())
}
```

---

## 6. Backup & Export

```bash
# Backup (safe during operation due to WAL mode)
cp ~/.talon/profiles/default/db/talon.db ~/talon-backup-$(date +%Y%m%d).db

# Or use SQLite's online backup API
sqlite3 talon.db ".backup talon-backup.db"

# Export session to markdown
talon export-session <session-id> > session.md
```
---

## Related Documents

### Depends On
- [Context & Memory Architecture](../02_Architecture/15_Context_And_Memory_Architecture.md)
- [SQLite & FTS5 in Rust](../07_Memory_System/55_SQLite_FTS5_In_Rust.md)

### See Also
- [FTS5 Search Deep Dive](../07_Memory_System/58_FTS5_Search_Deep_Dive.md)
- [Embedding Retrieval](../07_Memory_System/59_Embedding_Retrieval.md)
- [Session Management](../07_Memory_System/56_Session_Management.md)

