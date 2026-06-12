//! `/api/v1/secrets` — builtin vault management (criterion 15).
//!
//! Metadata only, in both directions where possible: `GET` lists
//! name/provider/created_at; `POST` accepts a value but the response — like
//! every other response from this module — structurally cannot carry one
//! (the response types have no value field). Mutations are admin-only via
//! the global role gate; a locked/absent vault is 503 with an actionable
//! error.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use super::WebState;

/// What the API exposes about a secret — there is deliberately no field a
/// value could travel in.
#[derive(serde::Serialize)]
pub struct SecretMetaOut {
    pub name: String,
    pub provider: String,
    pub created_at: Option<String>,
}

fn vault_unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "error": "builtin vault is locked or not initialized — run `talon init`, \
                      or unlock via the OS keychain / TALON_MASTER_KEY"
        })),
    )
        .into_response()
}

/// `GET /api/v1/secrets` — names + metadata, never values.
pub async fn list(State(state): State<WebState>) -> Response {
    let Some(vault) = &state.secret_vault else {
        return vault_unavailable();
    };
    match talon_secrets::SecretProvider::list(vault.as_ref()).await {
        Ok(metas) => Json(
            metas
                .into_iter()
                .map(|m| SecretMetaOut {
                    name: m.name,
                    provider: m.scheme,
                    created_at: m.created_at,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "secret list failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "secret store failure" })),
            )
                .into_response()
        }
    }
}

#[derive(serde::Deserialize)]
pub struct CreateRequest {
    pub name: String,
    pub value: String,
}

/// `POST /api/v1/secrets` — store (or replace) in the builtin vault. The
/// response echoes metadata only.
pub async fn create(State(state): State<WebState>, Json(req): Json<CreateRequest>) -> Response {
    let Some(vault) = &state.secret_vault else {
        return vault_unavailable();
    };

    let name = req.name.trim();
    let valid_name = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if !valid_name {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({
                "error": "secret name must be 1-64 chars of [A-Za-z0-9_.-]"
            })),
        )
            .into_response();
    }
    if req.value.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": "refusing to store an empty value" })),
        )
            .into_response();
    }

    match vault.set(name, &req.value).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "name": name,
                "provider": "builtin",
                "reference": format!("{{{{secret:{name}}}}}"),
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "secret create failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "secret store failure" })),
            )
                .into_response()
        }
    }
}

/// `DELETE /api/v1/secrets/{name}`.
pub async fn remove(State(state): State<WebState>, Path(name): Path<String>) -> Response {
    let Some(vault) = &state.secret_vault else {
        return vault_unavailable();
    };
    match vault.delete(&name).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no secret with this name" })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "secret delete failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "secret store failure" })),
            )
                .into_response()
        }
    }
}
