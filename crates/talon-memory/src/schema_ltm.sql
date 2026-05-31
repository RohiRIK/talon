-- Talon LTM schema (migration v2) — ADR 0008.
-- One DB: long-term memories live alongside sessions/messages in talon.db.
-- A fact, its FTS index, and its vector embedding all commit in one transaction.

-- ── Long-term memories ────────────────────────────────────────────────────────
-- category: user_preference | decision | fact | pattern | gotcha (validated in Rust)
-- importance: 1–5 · decay_score: starts at 1.0, lowered by the DecayEngine (2.5.10)
-- entities: JSON array of strings.

CREATE TABLE IF NOT EXISTS memories (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    content     TEXT    NOT NULL,
    category    TEXT    NOT NULL DEFAULT 'fact',
    importance  INTEGER NOT NULL DEFAULT 3 CHECK(importance BETWEEN 1 AND 5),
    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    accessed_at INTEGER NOT NULL DEFAULT (unixepoch()),
    decay_score REAL    NOT NULL DEFAULT 1.0,
    entities    TEXT    NOT NULL DEFAULT '[]'
);

CREATE INDEX IF NOT EXISTS idx_memories_category ON memories(category, importance);

-- ── FTS5 over memory content (external content, kept in sync via triggers) ─────

CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    content,
    category UNINDEXED,
    content='memories',
    content_rowid='id',
    tokenize='porter unicode61'
);

CREATE TRIGGER IF NOT EXISTS memories_fts_insert
    AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, content, category)
    VALUES (new.id, new.content, new.category);
END;

CREATE TRIGGER IF NOT EXISTS memories_fts_delete
    AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, content, category)
    VALUES ('delete', old.id, old.content, old.category);
END;

CREATE TRIGGER IF NOT EXISTS memories_fts_update
    AFTER UPDATE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, content, category)
    VALUES ('delete', old.id, old.content, old.category);
    INSERT INTO memories_fts(rowid, content, category)
    VALUES (new.id, new.content, new.category);
END;

-- ── Vector store (sqlite-vec vec0) — rowid mirrors memories.id ─────────────────
-- all-MiniLM-L6-v2 → 384 dims. Brute-force KNN (fine below ~100k vectors).

CREATE VIRTUAL TABLE IF NOT EXISTS vec_memories USING vec0(
    embedding float[384]
);
