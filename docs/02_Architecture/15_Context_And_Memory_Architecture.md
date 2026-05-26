# Context & Memory Architecture

> **Status:** ✅ Complete
> **Category:** Architecture

---

## 1. Memory Tiers

```
┌────────────────────────────────────────────────────────┐
│                   MEMORY TIERS                         │
│                                                        │
│  TIER 1 — Hot (In-Context)                            │
│  ┌──────────────────────────────────────────────────┐ │
│  │  System Prompt + MEMORY.md + USER.md + Skills   │ │
│  │  Last N messages from SQLite                    │ │
│  │  AGENTS.md (project context if workdir set)     │ │
│  └──────────────────────────────────────────────────┘ │
│                        ↕ injected every turn           │
│  TIER 2 — Warm (SQLite FTS5)                          │
│  ┌──────────────────────────────────────────────────┐ │
│  │  All past sessions, messages, tool outputs      │ │
│  │  Queryable via session_search tool              │ │
│  │  Tagged memory entries (mem0 style)             │ │
│  └──────────────────────────────────────────────────┘ │
│                        ↕ on-demand retrieval           │
│  TIER 3 — Cold (Filesystem)                           │
│  ┌──────────────────────────────────────────────────┐ │
│  │  SKILL.md files — procedural memory             │ │
│  │  MEMORY.md / USER.md / SOUL.md — markdown facts │ │
│  │  Profile config / secrets                       │ │
│  └──────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────┘
```

---

## 2. SQLite Schema

```sql
-- Sessions
CREATE TABLE sessions (
    id          TEXT PRIMARY KEY,  -- UUID
    title       TEXT,
    source      TEXT,              -- 'telegram', 'cli', 'cron', etc.
    profile     TEXT DEFAULT 'default',
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- Messages
CREATE TABLE messages (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT NOT NULL REFERENCES sessions(id),
    role        TEXT NOT NULL CHECK(role IN ('user','assistant','tool','system')),
    content     TEXT NOT NULL,  -- JSON ContentBlock array
    model       TEXT,
    tokens_in   INTEGER,
    tokens_out  INTEGER,
    created_at  INTEGER NOT NULL
);

-- FTS5 virtual table
CREATE VIRTUAL TABLE fts_messages USING fts5(
    content,
    session_id UNINDEXED,
    message_id UNINDEXED,
    tokenize = 'porter unicode61'
);

-- Memory entries (tagged key-value)
CREATE TABLE memory_entries (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    target      TEXT NOT NULL CHECK(target IN ('user','memory')),
    content     TEXT NOT NULL,
    tags        TEXT,  -- JSON array
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- Skills
CREATE TABLE skills (
    name        TEXT PRIMARY KEY,
    path        TEXT NOT NULL,
    category    TEXT,
    description TEXT,
    pinned      INTEGER DEFAULT 0,
    updated_at  INTEGER NOT NULL
);

-- Cron jobs
CREATE TABLE cron_jobs (
    id          TEXT PRIMARY KEY,
    name        TEXT,
    schedule    TEXT NOT NULL,
    prompt      TEXT NOT NULL,
    enabled     INTEGER DEFAULT 1,
    last_run    INTEGER,
    next_run    INTEGER,
    created_at  INTEGER NOT NULL
);
```

---

## 3. Context Window Budget

```rust
pub struct ContextBudget {
    pub total_tokens: u32,
    pub system_tokens: u32,       // target: ~15% of total
    pub history_tokens: u32,      // target: ~60% of total
    pub tool_output_tokens: u32,  // target: ~20% of total
    pub reserve: u32,             // target: ~5% — for response
}

impl ContextBudget {
    pub fn for_model(model: &str) -> Self {
        let total = match model {
            m if m.contains("claude-3-5") => 200_000,
            m if m.contains("gpt-4o") => 128_000,
            m if m.contains("gemini-1.5") => 1_000_000,
            _ => 32_000,
        };
        Self {
            total_tokens: total,
            system_tokens: (total as f32 * 0.15) as u32,
            history_tokens: (total as f32 * 0.60) as u32,
            tool_output_tokens: (total as f32 * 0.20) as u32,
            reserve: (total as f32 * 0.05) as u32,
        }
    }
}
```

When `history_tokens` budget is hit: trigger summarization of oldest 50% of turns, replace with a `[SUMMARY]` message.

---

## 4. MemoryStore API

```rust
pub struct MemoryStore {
    conn: Arc<Mutex<rusqlite::Connection>>,
    profile_dir: PathBuf,
}

impl MemoryStore {
    // Tier 1 — filesystem markdown
    pub async fn load_memory_md(&self) -> Result<String, MemoryError>;
    pub async fn save_memory_md(&self, content: &str) -> Result<(), MemoryError>;
    pub async fn load_user_md(&self) -> Result<String, MemoryError>;

    // Tier 2 — SQLite FTS5
    pub async fn search_sessions(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SessionSnippet>, MemoryError>;

    pub async fn get_session_window(
        &self,
        session_id: Uuid,
        around_message_id: i64,
        window: u32,
    ) -> Result<Vec<Message>, MemoryError>;

    // Memory entries
    pub async fn add_entry(
        &self,
        target: MemoryTarget,
        content: String,
        tags: Vec<String>,
    ) -> Result<(), MemoryError>;

    pub async fn list_entries(
        &self,
        target: MemoryTarget,
    ) -> Result<Vec<MemoryEntry>, MemoryError>;
}
```

---

## 5. Hot-Reload via `notify`

Skills are loaded at session start, but SKILL.md files change when the LLM edits them via `skill_manage`. Talon reloads them without restart:

```rust
use notify::{Watcher, RecursiveMode, Event};

pub fn start_skill_watcher(
    skills_dir: PathBuf,
    skill_store: Arc<SkillStore>,
) -> Result<(), notify::Error> {
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
    let mut watcher = notify::recommended_watcher(tx)?;
    watcher.watch(&skills_dir, RecursiveMode::Recursive)?;

    tokio::task::spawn_blocking(move || {
        for event in rx {
            if let Ok(ev) = event {
                if ev.kind.is_modify() || ev.kind.is_create() {
                    for path in &ev.paths {
                        if path.extension().map_or(false, |e| e == "md") {
                            skill_store.invalidate_cache(path);
                        }
                    }
                }
            }
        }
    });
    Ok(())
}
```

---

## 6. Context Assembly Pipeline

```
1. load_memory_md()        → ~500 tokens
2. load_user_md()          → ~200 tokens
3. list_skill_summaries()  → ~300 tokens (one-liner per skill)
4. load_agents_md()        → ~400 tokens (if workdir)
   ─────────────────────────────────────
   System prompt total     → ~1,400 tokens

5. load_recent_messages(session_id, budget=history_tokens)
   → Up to 60% of context window in actual conversation history

6. If total > 80% of model limit → trigger summarization

Final context slice sent to LLM provider.
```
---

## Related Documents

### Depends On
- [Cargo Workspace Design](12_Workspace_And_Crate_Structure.md)
- [SQLite & FTS5 in Rust](../07_Memory_System/55_SQLite_FTS5_In_Rust.md)

### Used By
- [Memory System (SQLite+FTS5)](../04_Core_Features/35_Memory_System_SQLite_FTS5.md)
- [User Modeling](../07_Memory_System/58a_User_Modeling.md)

### See Also
- [Session Management](../07_Memory_System/56_Session_Management.md)
- [Embedding Retrieval](../07_Memory_System/59_Embedding_Retrieval.md)
- [Cross-Session Context](../07_Memory_System/56a_Cross_Session_Context.md)

