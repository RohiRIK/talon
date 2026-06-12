//! Webhook triggers (criteria 25–27) — the Jenkins/n8n event-driven import.
//!
//! Two surfaces with very different trust levels:
//!
//! - `/api/v1/jobs/{id}/hooks` (bearer-authed, admin): register/list/revoke.
//!   The signing secret is generated here, stored in the builtin vault, and
//!   returned exactly once.
//! - `POST /hooks/{hook_id}` (PUBLIC — the binary's first unauthenticated
//!   endpoint): verified per delivery with HMAC-SHA256 over
//!   `"{timestamp}.{body}"` from `X-Talon-Signature` (hex) +
//!   `X-Talon-Timestamp` (unix seconds, ±[`TIMESTAMP_TOLERANCE_SECS`]),
//!   constant-time compare, per-hook rate limit, body size cap. A valid
//!   delivery queues an immediate run with the JSON payload exposed to the
//!   job as context; the run id is announced on the SSE feed at dispatch.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::WebState;

/// Replay window for `X-Talon-Timestamp` (Stripe-style).
pub const TIMESTAMP_TOLERANCE_SECS: i64 = 300;
/// Maximum delivery body (criterion 26 size cap).
pub const MAX_BODY_BYTES: usize = 64 * 1024;
/// Default per-hook deliveries per minute (criterion 27).
pub const DEFAULT_RATE_PER_MIN: u32 = 60;

type HmacSha256 = Hmac<Sha256>;

/// Fixed-window per-hook rate limiter. In-memory: limits reset on restart,
/// which is acceptable for an abuse brake (not a billing meter).
#[derive(Clone)]
pub struct HookLimiter {
    per_min: u32,
    windows: Arc<Mutex<HashMap<String, (Instant, u32)>>>,
}

impl HookLimiter {
    pub fn new(per_min: u32) -> Self {
        Self {
            per_min: per_min.max(1),
            windows: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// `true` when this delivery is within the hook's budget.
    pub fn allow(&self, hook_id: &str) -> bool {
        let mut windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let entry = windows.entry(hook_id.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0).as_secs() >= 60 {
            *entry = (now, 0);
        }
        entry.1 += 1;
        entry.1 <= self.per_min
    }
}

impl Default for HookLimiter {
    fn default() -> Self {
        Self::new(DEFAULT_RATE_PER_MIN)
    }
}

fn json_error(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

// ── Authed management surface (criterion 25) ─────────────────────────────────

/// `POST /api/v1/jobs/{id}/hooks` — the response is the only place the
/// signing secret ever appears.
pub async fn create(State(state): State<WebState>, Path(job_id): Path<String>) -> Response {
    let Some(vault) = &state.secret_vault else {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "webhooks need the builtin vault for signing secrets — run `talon init`",
        );
    };
    match state.cron.get(&job_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "no such job"),
        Err(e) => {
            tracing::error!(error = %e, "hook create: job lookup failed");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "store failure");
        }
    }

    let hook = match state.hooks.create(&job_id).await {
        Ok(hook) => hook,
        Err(e) => {
            tracing::error!(error = %e, "hook create failed");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "store failure");
        }
    };

    let secret = format!(
        "whsec_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    if let Err(e) = vault.set(&hook.secret_name, &secret).await {
        tracing::error!(error = %e, "hook secret store failed");
        let _ = state.hooks.revoke(&hook.id).await;
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "secret store failure");
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "hook_id": hook.id,
            "job_id": hook.job_id,
            "url": format!("/hooks/{}", hook.id),
            "secret": secret,
            "signature": "X-Talon-Signature: hex(hmac_sha256(secret, \"{X-Talon-Timestamp}.{body}\"))",
            "note": "store this secret now — it is never shown again"
        })),
    )
        .into_response()
}

/// `GET /api/v1/jobs/{id}/hooks` — no secrets, ever.
pub async fn list(State(state): State<WebState>, Path(job_id): Path<String>) -> Response {
    match state.hooks.list_for_job(&job_id).await {
        Ok(hooks) => Json(
            hooks
                .into_iter()
                .map(|h| {
                    serde_json::json!({
                        "hook_id": h.id,
                        "job_id": h.job_id,
                        "url": format!("/hooks/{}", h.id),
                        "created_at": h.created_at,
                        "revoked": h.revoked,
                    })
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "hook list failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "store failure")
        }
    }
}

/// `DELETE /api/v1/hooks/{hook_id}` — revoke; subsequent deliveries 404.
pub async fn revoke(State(state): State<WebState>, Path(hook_id): Path<String>) -> Response {
    match state.hooks.revoke(&hook_id).await {
        Ok(true) => {
            // Best-effort secret cleanup; a dangling vault entry is harmless.
            if let Some(vault) = &state.secret_vault {
                let _ = vault.delete(&format!("webhook/{hook_id}")).await;
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => json_error(StatusCode::NOT_FOUND, "no active hook with this id"),
        Err(e) => {
            tracing::error!(error = %e, "hook revoke failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "store failure")
        }
    }
}

// ── Public delivery surface (criteria 26–27) ─────────────────────────────────

/// `POST /hooks/{hook_id}` — no bearer auth; HMAC is the credential.
pub async fn deliver(
    State(state): State<WebState>,
    Path(hook_id): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    // Order matters: existence → rate → size → signature. Rate-limiting
    // before signature verification keeps HMAC work off an abusive path.
    let hook = match state.hooks.get_active(&hook_id).await {
        Ok(Some(hook)) => hook,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "unknown or revoked hook"),
        Err(e) => {
            tracing::error!(error = %e, "hook delivery lookup failed");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "store failure");
        }
    };

    if !state.hook_limiter.allow(&hook_id) {
        return json_error(StatusCode::TOO_MANY_REQUESTS, "hook rate limit exceeded");
    }
    if body.len() > MAX_BODY_BYTES {
        return json_error(StatusCode::PAYLOAD_TOO_LARGE, "payload exceeds 64KB");
    }

    let Some(vault) = &state.secret_vault else {
        return json_error(StatusCode::SERVICE_UNAVAILABLE, "vault unavailable");
    };
    // Direct ref construction: hook secret names contain `/`, which the
    // `{{secret:NAME}}` textual form deliberately rejects.
    let sref = talon_secrets::SecretRef {
        scheme: talon_secrets::BUILTIN_SCHEME.to_string(),
        path: hook.secret_name.clone(),
        key: None,
        raw: hook.secret_name.clone(),
    };
    let secret = match talon_secrets::SecretProvider::get(vault.as_ref(), &sref).await {
        Ok(secret) => secret,
        Err(e) => {
            tracing::error!(error = %e, "hook signing secret unavailable");
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "signing secret unavailable",
            );
        }
    };

    if let Err(reason) = verify_signature(&headers, &body, secret.expose()) {
        return json_error(StatusCode::UNAUTHORIZED, reason);
    }

    let payload = (!body.is_empty()).then(|| String::from_utf8_lossy(&body).into_owned());
    if !state.sched.trigger_webhook(&hook.job_id, payload).await {
        return json_error(StatusCode::SERVICE_UNAVAILABLE, "scheduler is not running");
    }

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "status": "queued",
            "job_id": hook.job_id,
            "hook_id": hook.id,
        })),
    )
        .into_response()
}

/// HMAC-SHA256 over `"{timestamp}.{body}"`; hex signature; replay window.
/// `Mac::verify_slice` is constant-time.
fn verify_signature(headers: &HeaderMap, body: &[u8], secret: &str) -> Result<(), &'static str> {
    let ts_raw = headers
        .get("x-talon-timestamp")
        .and_then(|v| v.to_str().ok())
        .ok_or("missing X-Talon-Timestamp")?;
    let ts: i64 = ts_raw.parse().map_err(|_| "malformed X-Talon-Timestamp")?;
    let now = chrono::Utc::now().timestamp();
    if (now - ts).abs() > TIMESTAMP_TOLERANCE_SECS {
        return Err("timestamp outside the replay window");
    }

    let sig_hex = headers
        .get("x-talon-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or("missing X-Talon-Signature")?;
    let sig = decode_hex(sig_hex).ok_or("malformed X-Talon-Signature")?;

    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| "signing secret unusable")?;
    mac.update(ts_raw.as_bytes());
    mac.update(b".");
    mac.update(body);
    mac.verify_slice(&sig).map_err(|_| "signature mismatch")
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Test/client helper: produce the expected signature for a delivery.
// Invariant: HMAC accepts keys of any length — new_from_slice cannot fail.
#[allow(clippy::expect_used)]
pub fn sign(secret: &str, timestamp: &str, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac accepts any key len");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    let bytes = mac.finalize().into_bytes();
    let mut out = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn limiter_enforces_per_minute_budget() {
        let limiter = HookLimiter::new(3);
        assert!(limiter.allow("h1"));
        assert!(limiter.allow("h1"));
        assert!(limiter.allow("h1"));
        assert!(!limiter.allow("h1"), "4th in the window is rejected");
        assert!(limiter.allow("h2"), "independent per hook");
    }

    #[test]
    fn sign_and_verify_roundtrip_and_tamper_detection() {
        let secret = "whsec_test";
        let ts = chrono::Utc::now().timestamp().to_string();
        let body = br#"{"event":"push"}"#;

        let mut headers = HeaderMap::new();
        headers.insert("x-talon-timestamp", ts.parse().expect("hv"));
        headers.insert(
            "x-talon-signature",
            sign(secret, &ts, body).parse().expect("hv"),
        );
        assert!(verify_signature(&headers, body, secret).is_ok());

        // Tampered body fails.
        assert!(verify_signature(&headers, b"{}", secret).is_err());
        // Wrong secret fails.
        assert!(verify_signature(&headers, body, "whsec_other").is_err());
    }

    #[test]
    fn stale_timestamp_rejected() {
        let secret = "whsec_test";
        let stale = (chrono::Utc::now().timestamp() - TIMESTAMP_TOLERANCE_SECS - 10).to_string();
        let body = b"x";
        let mut headers = HeaderMap::new();
        headers.insert("x-talon-timestamp", stale.parse().expect("hv"));
        headers.insert(
            "x-talon-signature",
            sign(secret, &stale, body).parse().expect("hv"),
        );
        let err = verify_signature(&headers, body, secret).expect_err("stale");
        assert!(err.contains("replay"));
    }
}
