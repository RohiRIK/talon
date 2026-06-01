-- Migration v4 — cron scheduler jobs.
-- Single source of truth for the self-rolled tick-loop scheduler (SPEC §4.2).
-- All timestamps are UTC RFC3339 with a 'Z' suffix so lexicographic string
-- comparison on next_run is a valid chronological comparison.

CREATE TABLE IF NOT EXISTS cron_jobs (
    id            TEXT PRIMARY KEY,
    name          TEXT,
    schedule      TEXT    NOT NULL,                       -- JSON CronSchedule
    prompt        TEXT    NOT NULL,
    session_id    TEXT    NOT NULL,                       -- stable per job → LTM continuity
    deliver_to    TEXT    NOT NULL DEFAULT 'origin',      -- origin/local/all/platform:chat:thread
    context_from  TEXT    NOT NULL DEFAULT '[]',          -- JSON array of upstream job ids (DAG)
    granted_scope TEXT    NOT NULL DEFAULT '{"tools":[],"bash_patterns":[]}', -- §4.4 wizard grant
    enabled       INTEGER NOT NULL DEFAULT 1,             -- 1/0
    tz            TEXT    NOT NULL DEFAULT 'UTC',          -- IANA timezone
    repeat        INTEGER,                                -- NULL=infinite, 1=one-shot, n=n runs
    run_count     INTEGER NOT NULL DEFAULT 0,
    last_run      TEXT,                                   -- UTC RFC3339, nullable
    last_output   TEXT,                                   -- for context_from injection downstream
    next_run      TEXT,                                   -- UTC RFC3339, computed by croner
    created_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- Hot path: the scheduler's due-query filters on enabled + next_run every tick.
CREATE INDEX IF NOT EXISTS idx_cron_due ON cron_jobs (enabled, next_run);
