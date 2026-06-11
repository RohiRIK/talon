-- Migration v5 — per-run execution history for cron jobs (web console, SPEC Phase 7).
-- One row per execution attempt. `cron_jobs.last_run`/`last_output` semantics are
-- unchanged (§4.5 crash policy); this table records attempts *alongside* them so
-- the web console can show failures, timelines, and per-run transcripts.
-- Timestamps are UTC RFC3339 with a 'Z' suffix, same convention as cron_jobs.

CREATE TABLE IF NOT EXISTS cron_runs (
    id          TEXT PRIMARY KEY,
    job_id      TEXT NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
    started_at  TEXT NOT NULL,
    finished_at TEXT,
    status      TEXT NOT NULL CHECK (status IN
                ('running','success','failure','timeout','skipped','denied')),
    output      TEXT,
    error       TEXT,
    events_json TEXT
);

-- Hot path: run history per job, newest first.
CREATE INDEX IF NOT EXISTS idx_cron_runs_job ON cron_runs (job_id, started_at DESC);
