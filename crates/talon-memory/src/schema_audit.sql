-- Migration v10 — audit log (Phase 8 "Flow Cottage", criterion 32).
-- One row per mutating /api/v1 request. `token_fp` is the first 8 hex chars
-- of SHA-256(token) — joinable to api_tokens.token_hash prefixes, never the
-- token itself. `target_id` is the path's resource segment when present.

CREATE TABLE IF NOT EXISTS audit_log (
    id        TEXT PRIMARY KEY,
    ts        TEXT NOT NULL,
    token_fp  TEXT NOT NULL,
    method    TEXT NOT NULL,
    path      TEXT NOT NULL,
    target_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_log (ts DESC);
