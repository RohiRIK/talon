-- Migration v7 — named API tokens with roles (Phase 8 "Flow Cottage", criteria 4-6).
-- Only the SHA-256 hex of a token is stored; the raw token is shown exactly once
-- at creation. `revoked_at` is a tombstone — revoked rows stay for the audit trail.
-- Timestamps are UTC RFC3339 with a 'Z' suffix, same convention as cron_jobs.

CREATE TABLE IF NOT EXISTS api_tokens (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    token_hash  TEXT NOT NULL UNIQUE,
    role        TEXT NOT NULL CHECK (role IN ('admin','viewer')),
    created_at  TEXT NOT NULL,
    last_used   TEXT,
    revoked_at  TEXT
);

CREATE INDEX IF NOT EXISTS idx_api_tokens_hash ON api_tokens (token_hash) WHERE revoked_at IS NULL;
