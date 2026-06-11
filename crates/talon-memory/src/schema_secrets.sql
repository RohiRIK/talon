-- Migration v6 — builtin encrypted vault (talon-secrets, Phase 8 "Flow Cottage").
-- Envelope encryption: `ciphertext` is the secret value sealed AES-256-GCM with a
-- per-secret data key (DEK); `wrapped_dek` is that DEK sealed with the master key.
-- The master key never touches this database — it lives wrapped in the OS keychain
-- and/or the argon2id recovery blob (~/.talon/master.key.enc).
-- No plaintext secret material is ever written here.
-- Timestamps are UTC RFC3339 with a 'Z' suffix, same convention as cron_jobs.

CREATE TABLE IF NOT EXISTS secrets (
    name        TEXT PRIMARY KEY,
    ciphertext  BLOB NOT NULL,
    nonce       BLOB NOT NULL,
    wrapped_dek BLOB NOT NULL,
    dek_nonce   BLOB NOT NULL,
    provider    TEXT NOT NULL DEFAULT 'builtin',
    created_at  TEXT NOT NULL
);
