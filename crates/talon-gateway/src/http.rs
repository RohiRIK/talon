use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::post,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use talon_core::events::AgentEvent;

use crate::{Gateway, GatewayContext, GatewayError, RenderMode};

/// HTTP gateway — single `POST /v1/messages` endpoint.
///
/// Build this first (no bot token required) to verify the agent loop works
/// before wiring Telegram and TUI.
pub struct HttpGateway {
    ctx: Arc<GatewayContext>,
    addr: SocketAddr,
}

impl HttpGateway {
    pub fn new(ctx: Arc<GatewayContext>, addr: SocketAddr) -> Self {
        Self { ctx, addr }
    }
}

impl Gateway for HttpGateway {
    fn name(&self) -> &str {
        "http"
    }

    fn render_mode(&self) -> RenderMode {
        RenderMode::Plain
    }

    fn run(&self) -> Pin<Box<dyn Future<Output = Result<(), GatewayError>> + Send + '_>> {
        Box::pin(async move {
            let app_state = Arc::clone(&self.ctx);

            let app = Router::new()
                .route("/v1/messages", post(handle_message))
                .with_state(app_state);

            let listener = tokio::net::TcpListener::bind(self.addr)
                .await
                .map_err(GatewayError::Io)?;

            tracing::info!("HTTP gateway listening on {}", self.addr);

            axum::serve(listener, app)
                .await
                .map_err(GatewayError::Io)?;

            Ok(())
        })
    }
}

// ── Request / Response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct MessageRequest {
    pub content: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub content: String,
    pub session_id: String,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

// ── Handler ───────────────────────────────────────────────────────────────────

async fn handle_message(
    State(ctx): State<Arc<GatewayContext>>,
    Json(req): Json<MessageRequest>,
) -> Result<Json<MessageResponse>, (StatusCode, Json<ErrorBody>)> {
    let session_id = req
        .session_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(64);

    // Collect the last text from the agent.
    let collector = tokio::spawn(async move {
        let mut last_text = String::new();
        while let Some(event) = event_rx.recv().await {
            match event {
                AgentEvent::Text { content } => last_text = content,
                AgentEvent::Completed | AgentEvent::Failed(_) => break,
                AgentEvent::ApprovalRequested { tx, .. } => {
                    // HTTP gateway auto-approves Safe tools; Dangerous are denied.
                    // Callers that need approval should use the TUI or CLI gateway.
                    tx.send(false).ok();
                }
                _ => {}
            }
        }
        last_text
    });

    let mut agent = ctx.build_agent(event_tx);

    agent
        .run(&session_id, req.content)
        .await
        .map_err(|e| agent_err(e.to_string()))?;

    let content = collector.await.unwrap_or_default();

    Ok(Json(MessageResponse {
        content,
        session_id,
    }))
}

fn agent_err(msg: String) -> (StatusCode, Json<ErrorBody>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorBody { error: msg }),
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use talon_llm::MockProvider;
    use tower::ServiceExt;

    fn make_ctx() -> Arc<GatewayContext> {
        Arc::new(GatewayContext::new(Arc::new(MockProvider::text(
            "hello from http",
            "end_turn",
        ))))
    }

    fn make_app(ctx: Arc<GatewayContext>) -> Router {
        Router::new()
            .route("/v1/messages", post(handle_message))
            .with_state(ctx)
    }

    #[test]
    fn http_gateway_name() {
        let gw = HttpGateway::new(
            make_ctx(),
            "127.0.0.1:0".parse().expect("addr"),
        );
        assert_eq!(gw.name(), "http");
    }

    #[tokio::test]
    async fn post_messages_returns_200() {
        let app = make_app(make_ctx());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/messages")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"content":"hi"}"#))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn post_messages_returns_content() {
        let app = make_app(make_ctx());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/messages")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"content":"hello"}"#))
                    .expect("request"),
            )
            .await
            .expect("response");

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json");

        assert!(json["content"].as_str().is_some());
        assert!(json["session_id"].as_str().is_some());
    }

    #[tokio::test]
    async fn post_messages_accepts_session_id() {
        let app = make_app(make_ctx());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/messages")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"content":"hi","session_id":"test-session"}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(json["session_id"].as_str(), Some("test-session"));
    }

    #[tokio::test]
    async fn post_messages_bad_json_returns_4xx() {
        let app = make_app(make_ctx());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/messages")
                    .header("content-type", "application/json")
                    .body(Body::from("not json"))
                    .expect("request"),
            )
            .await
            .expect("response");

        // Axum returns 400 for syntactically invalid JSON (malformed body).
        assert!(
            resp.status().is_client_error(),
            "expected 4xx, got {}",
            resp.status()
        );
    }
}
