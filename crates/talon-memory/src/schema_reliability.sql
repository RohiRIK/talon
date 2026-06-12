-- Migration v9 — run reliability (Phase 8 "Flow Cottage", criteria 28-30).
-- Per-job retry policy and an optional error-handler job. Defaults preserve
-- existing behavior exactly: retry_max=0 means one attempt, on_failure NULL
-- means failures trigger nothing.

ALTER TABLE cron_jobs ADD COLUMN retry_max INTEGER NOT NULL DEFAULT 0;
ALTER TABLE cron_jobs ADD COLUMN on_failure TEXT;
