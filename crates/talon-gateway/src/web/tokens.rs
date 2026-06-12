//! `/api/v1/tokens` — named API token management (criterion 4).
//!
//! Admin-only in both directions: the role-gate middleware blocks viewer
//! mutations globally, and `list` additionally refuses viewers (token
//! existence is itself sensitive). The raw token appears exactly once, in
//! the `POST` response; only its SHA-256 lives in the database.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use talon_memory::{MemoryError, TokenRole};

use super::{AuthIdentity, WebState};

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({ "error": "admin token required" })),
    )
        .into_response()
}

fn internal(e: MemoryError) -> Response {
    tracing::error!(error = %e, "token store failure");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": "token store failure" })),
    )
        .into_response()
}

/// `GET /api/v1/tokens` — metadata only, never hashes or raw tokens.
pub async fn list(
    State(state): State<WebState>,
    Extension(identity): Extension<AuthIdentity>,
) -> Response {
    if identity.role != TokenRole::Admin {
        return forbidden();
    }
    match state.tokens.list().await {
        Ok(metas) => Json(metas).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(serde::Deserialize)]
pub struct CreateRequest {
    pub name: String,
    pub role: TokenRole,
}

/// `POST /api/v1/tokens` — the response is the only place the raw token
/// ever exists (criterion 4).
pub async fn create(State(state): State<WebState>, Json(req): Json<CreateRequest>) -> Response {
    let name = req.name.trim();
    if name.is_empty() || name.len() > 64 {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": "token name must be 1-64 characters" })),
        )
            .into_response();
    }

    match state.tokens.create(name, req.role).await {
        Ok(raw) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "name": name,
                "role": req.role,
                "token": raw,
                "note": "store this token now — it is never shown again"
            })),
        )
            .into_response(),
        // UNIQUE(name) violation → conflict, anything else → 500.
        Err(MemoryError::Rusqlite(e)) if e.to_string().contains("UNIQUE constraint failed") => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "a token with this name already exists" })),
        )
            .into_response(),
        Err(e) => internal(e),
    }
}

/// `DELETE /api/v1/tokens/{name}` — revoke (tombstone).
pub async fn revoke(State(state): State<WebState>, Path(name): Path<String>) -> Response {
    match state.tokens.revoke(&name).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no active token with this name" })),
        )
            .into_response(),
        Err(e) => internal(e),
    }
}
