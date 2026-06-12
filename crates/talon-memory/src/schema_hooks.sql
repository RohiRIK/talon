-- Migration v8 — webhook triggers (Phase 8 "Flow Cottage", criteria 25-27).
-- A hook is a child resource of a job: the public delivery endpoint
-- POST /hooks/{id} verifies an HMAC signature whose secret lives in the
-- builtin vault under `secret_name` (never in this table). Revocation is a
-- tombstone. `cron_runs` gains provenance: `fired_by` ('cron' | 'manual' |
-- 'webhook' — named fired_by because TRIGGER is a SQL keyword) and `attempt`
-- (retry chains, Phase 9).

CREATE TABLE IF NOT EXISTS webhooks (
    id          TEXT PRIMARY KEY,
    job_id      TEXT NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
    secret_name TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    revoked_at  TEXT
);

CREATE INDEX IF NOT EXISTS idx_webhooks_job ON webhooks (job_id);

ALTER TABLE cron_runs ADD COLUMN fired_by TEXT NOT NULL DEFAULT 'cron';
ALTER TABLE cron_runs ADD COLUMN attempt INTEGER NOT NULL DEFAULT 1;
