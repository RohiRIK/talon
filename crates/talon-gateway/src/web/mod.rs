//! Web console API (`/api/v1`) — SPEC Phase 7.
//!
//! A thin HTTP surface over the existing primitives: `CronStore` (jobs),
//! `RunStore` (per-run history), `SchedulerHandle` (manual triggers),
//! a `broadcast` feed of [`RunEvent`]s (SSE), and the [`ApprovalBroker`]
//! (unattended-escalation resolution). No scheduling logic lives here.
//!
//! Auth: a single bearer token, required on every route. The SSE endpoint
//! additionally accepts `?token=` because `EventSource` cannot set headers.
//! Construction is fail-closed: a [`WebState`] cannot exist without a
//! non-empty token, so unauthenticated mounting is unrepresentable.

pub mod approvals;
pub mod flows;
pub mod handlers;
pub mod logs;
#[cfg(feature = "web-ui")]
pub mod serve_ui;
pub mod sse;
pub mod tokens;

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use tokio::sync::broadcast;

use talon_core::scheduler::{RunEvent, SchedulerHandle};
use talon_memory::{CronStore, RunStore, TokenRole, TokenStore};

use crate::GatewayContext;
pub use approvals::{ApprovalBroker, PendingApproval};

/// Capacity of the run-event broadcast feed. Slow SSE subscribers lose old
/// events rather than blocking the scheduler.
pub const EVENT_CHANNEL_CAP: usize = 256;

/// Who is making the request — resolved by the auth middleware and available
/// to handlers via request extensions (criterion 5).
#[derive(Debug, Clone, serde::Serialize)]
pub struct AuthIdentity {
    pub name: String,
    pub role: TokenRole,
}

/// Identity assigned to the legacy `[gateway] api_token` (criterion 6:
/// implicit admin during the deprecation window).
pub const LEGACY_TOKEN_NAME: &str = "legacy";

/// Type-erased Prometheus render closure — keeps the exporter crate out of
/// the gateway's dependency tree (the binary installs the recorder).
pub type MetricsRender = Arc<dyn Fn() -> String + Send + Sync>;

/// Everything the `/api/v1` handlers need, cheap to clone per request.
#[derive(Clone)]
pub struct WebState {
    pub ctx: Arc<GatewayContext>,
    pub cron: CronStore,
    pub runs: RunStore,
    pub sched: SchedulerHandle,
    pub events: broadcast::Sender<RunEvent>,
    pub approvals: ApprovalBroker,
    pub tokens: TokenStore,
    pub log_ring: Option<Arc<logs::LogRing>>,
    pub metrics_render: Option<MetricsRender>,
    token: Arc<str>,
}

impl WebState {
    /// Fail-closed constructor: an empty/whitespace token is a config error,
    /// not an open server.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ctx: Arc<GatewayContext>,
        cron: CronStore,
        runs: RunStore,
        sched: SchedulerHandle,
        events: broadcast::Sender<RunEvent>,
        approvals: ApprovalBroker,
        tokens: TokenStore,
        token: &str,
    ) -> Result<Self, crate::GatewayError> {
        let token = token.trim();
        if token.is_empty() {
            return Err(crate::GatewayError::Config(
                "web console requires a non-empty [gateway] api_token".to_string(),
            ));
        }
        Ok(Self {
            ctx,
            cron,
            runs,
            sched,
            events,
            approvals,
            tokens,
            log_ring: None,
            metrics_render: None,
            token: Arc::from(token),
        })
    }

    /// Attach the live log ring (criterion 20).
    pub fn with_log_ring(mut self, ring: Arc<logs::LogRing>) -> Self {
        self.log_ring = Some(ring);
        self
    }

    /// Attach the Prometheus render closure (criterion 19).
    pub fn with_metrics(mut self, render: MetricsRender) -> Self {
        self.metrics_render = Some(render);
        self
    }
}

/// The `/api/v1` router with bearer-token auth applied to every route.
/// Nest under `/api/v1`: `Router::new().nest("/api/v1", api_router(state))`.
pub fn api_router(state: WebState) -> Router {
    Router::new()
        .route("/jobs", get(handlers::list_jobs).post(handlers::create_job))
        .route(
            "/jobs/{id}",
            get(handlers::get_job)
                .patch(handlers::patch_job)
                .delete(handlers::delete_job),
        )
        .route("/jobs/{id}/trigger", post(handlers::trigger_job))
        .route("/jobs/{id}/runs", get(handlers::list_runs))
        .route("/runs/{id}", get(handlers::get_run))
        .route("/graph", get(handlers::graph))
        .route("/flows/plan", post(flows::plan))
        .route("/flows", post(flows::commit))
        .route("/approvals", get(handlers::list_approvals))
        .route("/approvals/{call_id}", post(handlers::resolve_approval))
        .route("/events", get(sse::events))
        .route("/me", get(me))
        .route("/tokens", get(tokens::list).post(tokens::create))
        .route("/tokens/{name}", delete(tokens::revoke))
        .route("/logs/tail", get(logs::tail))
        .layer(middleware::from_fn_with_state(state.clone(), require_token))
        .layer(middleware::from_fn(request_id))
        .with_state(state)
}

/// Token-protected Prometheus endpoint, mounted at the root (`/metrics`) so
/// scrape configs use the conventional path (criterion 19).
pub fn metrics_router(state: WebState) -> Router {
    Router::new()
        .route("/metrics", get(metrics_handler))
        .layer(middleware::from_fn_with_state(state.clone(), require_token))
        .with_state(state)
}

async fn metrics_handler(State(state): State<WebState>) -> Response {
    match &state.metrics_render {
        Some(render) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
            render(),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Correlation middleware (criterion 17): every request gets a `request_id`,
/// echoed in the `x-request-id` response header and attached to the request's
/// tracing span.
async fn request_id(req: Request, next: Next) -> Response {
    use tracing::Instrument as _;

    let id = uuid::Uuid::new_v4().simple().to_string();
    let span = tracing::info_span!("http", request_id = %id);

    let mut resp = next.run(req).instrument(span).await;
    if let Ok(value) = axum::http::HeaderValue::from_str(&id) {
        resp.headers_mut().insert("x-request-id", value);
    }
    resp
}

/// `GET /api/v1/me` — the caller's token name and role (criterion 5; the
/// console login view validates against this).
async fn me(Extension(identity): Extension<AuthIdentity>) -> Json<AuthIdentity> {
    Json(identity)
}

/// Auth gate (criteria 4–6). Resolves an [`AuthIdentity`] from either the
/// legacy `[gateway] api_token` (implicit admin) or a named token in
/// `api_tokens`, then enforces the role: viewers get GET/HEAD only. The SSE
/// path additionally accepts `?token=<token>` (EventSource limitation —
/// localhost-default bind and no URI logging on that path bound the risk).
async fn require_token(State(state): State<WebState>, mut req: Request, next: Next) -> Response {
    let header_token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);

    let query_token = if req.uri().path().ends_with("/events") {
        req.uri().query().and_then(|q| {
            q.split('&')
                .find_map(|pair| pair.strip_prefix("token=").map(str::to_string))
        })
    } else {
        None
    };

    let mut identity: Option<AuthIdentity> = None;
    for presented in [header_token, query_token].into_iter().flatten() {
        if token_eq(&presented, state.token.as_ref()) {
            identity = Some(AuthIdentity {
                name: LEGACY_TOKEN_NAME.to_string(),
                role: TokenRole::Admin,
            });
            break;
        }
        match state.tokens.verify(&presented).await {
            Ok(Some((name, role))) => {
                identity = Some(AuthIdentity { name, role });
                break;
            }
            Ok(None) => {}
            Err(e) => tracing::error!(error = %e, "token verification failed"),
        }
    }

    let Some(identity) = identity else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "unauthorized" })),
        )
            .into_response();
    };

    // Role gate (criterion 5): mutating methods are admin-only.
    let read_only = matches!(
        req.method(),
        &axum::http::Method::GET | &axum::http::Method::HEAD
    );
    if !read_only && identity.role == TokenRole::Viewer {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "viewer tokens are read-only" })),
        )
            .into_response();
    }

    req.extensions_mut().insert(identity);
    next.run(req).await
}

/// Constant-time-ish comparison: never short-circuits on the first differing
/// byte once lengths match. (Length itself is not secret-graded here.)
fn token_eq(given: &str, expected: &str) -> bool {
    if given.len() != expected.len() {
        return false;
    }
    given
        .bytes()
        .zip(expected.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt;

    use talon_core::scheduler::{JobOutcome, JobRunner, Scheduler};
    use talon_llm::MockProvider;
    use talon_memory::{CronJob, CronSchedule, Database, RunStatus};

    use super::*;

    struct NoopRunner;
    impl JobRunner for NoopRunner {
        fn run(&self, _job: CronJob) -> Pin<Box<dyn Future<Output = JobOutcome> + Send + '_>> {
            Box::pin(async { JobOutcome::ok(None) })
        }
    }

    const TOKEN: &str = "test-token-123";

    async fn make_state() -> WebState {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        let cron = CronStore::new(Arc::clone(&db));
        let tokens = TokenStore::new(Arc::clone(&db));
        let runs = RunStore::new(db);
        let scheduler = Scheduler::new(cron.clone(), Arc::new(NoopRunner));
        let sched = scheduler.handle();
        // Keep the scheduler alive for the test so the handle stays connected.
        std::mem::forget(scheduler);
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAP);
        let ctx = Arc::new(GatewayContext::new(Arc::new(MockProvider::text(
            "ok", "end_turn",
        ))));
        WebState::new(
            ctx,
            cron,
            runs,
            sched,
            events,
            ApprovalBroker::new(),
            tokens,
            TOKEN,
        )
        .expect("state")
    }

    fn app(state: WebState) -> Router {
        Router::new().nest("/api/v1", api_router(state))
    }

    fn authed(method: &str, uri: &str, body: Option<&str>) -> HttpRequest<Body> {
        let builder = HttpRequest::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("content-type", "application/json");
        match body {
            Some(b) => builder.body(Body::from(b.to_string())).expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        }
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json")
    }

    // ── Auth (criterion 8) ──────────────────────────────────────────────────

    #[tokio::test]
    async fn requests_without_token_get_401() {
        let state = make_state().await;
        for (method, uri) in [
            ("GET", "/api/v1/jobs"),
            ("POST", "/api/v1/jobs"),
            ("GET", "/api/v1/graph"),
            ("GET", "/api/v1/events"),
            ("GET", "/api/v1/approvals"),
        ] {
            let req = HttpRequest::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::empty())
                .expect("request");
            let resp = app(state.clone()).oneshot(req).await.expect("response");
            assert_eq!(
                resp.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {uri} must 401 without token"
            );
        }
    }

    #[tokio::test]
    async fn wrong_token_gets_401() {
        let state = make_state().await;
        let req = HttpRequest::builder()
            .method("GET")
            .uri("/api/v1/jobs")
            .header("authorization", "Bearer wrong-token-999")
            .body(Body::empty())
            .expect("request");
        let resp = app(state).oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn bearer_token_grants_access() {
        let state = make_state().await;
        let resp = app(state)
            .oneshot(authed("GET", "/api/v1/jobs", None))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn sse_accepts_query_token_other_routes_do_not() {
        let state = make_state().await;

        let sse = HttpRequest::builder()
            .method("GET")
            .uri(format!("/api/v1/events?token={TOKEN}"))
            .body(Body::empty())
            .expect("request");
        let resp = app(state.clone()).oneshot(sse).await.expect("response");
        assert_eq!(resp.status(), StatusCode::OK, "SSE honors ?token=");
        assert!(
            resp.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("text/event-stream")),
            "SSE content type"
        );

        let other = HttpRequest::builder()
            .method("GET")
            .uri(format!("/api/v1/jobs?token={TOKEN}"))
            .body(Body::empty())
            .expect("request");
        let resp = app(state).oneshot(other).await.expect("response");
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "query token is SSE-only"
        );
    }

    #[test]
    fn empty_token_is_rejected_at_construction() {
        // Fail-closed: WebState cannot be built with a blank token, so an
        // unauthenticated /api/v1 cannot exist.
        let (events, _) = broadcast::channel(1);
        let db = Database::open(":memory:").expect("open");
        let db = Arc::new(db);
        let cron = CronStore::new(Arc::clone(&db));
        let tokens = TokenStore::new(Arc::clone(&db));
        let runs = RunStore::new(db);
        let scheduler = Scheduler::new(cron.clone(), Arc::new(NoopRunner));
        let ctx = Arc::new(GatewayContext::new(Arc::new(MockProvider::text(
            "ok", "end_turn",
        ))));
        let result = WebState::new(
            ctx,
            cron,
            runs,
            scheduler.handle(),
            events,
            ApprovalBroker::new(),
            tokens,
            "   ",
        );
        assert!(result.is_err());
    }

    // ── Jobs CRUD (criteria 1, 9) ───────────────────────────────────────────

    #[tokio::test]
    async fn create_list_get_patch_delete_roundtrip() {
        let state = make_state().await;

        // Create.
        let resp = app(state.clone())
            .oneshot(authed(
                "POST",
                "/api/v1/jobs",
                Some(r#"{"prompt":"summarize inbox","schedule":"daily","name":"brief","deliver_to":"telegram:me"}"#),
            ))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::CREATED);
        let created = body_json(resp).await;
        let id = created["id"].as_str().expect("id").to_string();
        assert_eq!(created["name"], "brief");
        assert!(created["next_run"].as_str().is_some(), "next_run computed");
        assert!(created["latest_run"].is_null(), "never run yet");

        // List shows it.
        let resp = app(state.clone())
            .oneshot(authed("GET", "/api/v1/jobs", None))
            .await
            .expect("response");
        let list = body_json(resp).await;
        assert_eq!(list.as_array().expect("array").len(), 1);

        // Patch: disable.
        let resp = app(state.clone())
            .oneshot(authed(
                "PATCH",
                &format!("/api/v1/jobs/{id}"),
                Some(r#"{"enabled":false}"#),
            ))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        let patched = body_json(resp).await;
        assert_eq!(patched["enabled"], false);
        assert!(patched["next_run"].is_null(), "disable clears next_run");

        // Delete.
        let resp = app(state.clone())
            .oneshot(authed("DELETE", &format!("/api/v1/jobs/{id}"), None))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let resp = app(state)
            .oneshot(authed("GET", &format!("/api/v1/jobs/{id}"), None))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn create_with_bad_cron_is_422_and_writes_nothing() {
        let state = make_state().await;
        let resp = app(state.clone())
            .oneshot(authed(
                "POST",
                "/api/v1/jobs",
                Some(r#"{"prompt":"x","schedule":"not a schedule"}"#),
            ))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(state.cron.list().await.expect("list").is_empty());
    }

    #[tokio::test]
    async fn create_without_scope_predicts_from_prompt() {
        let state = make_state().await;
        let resp = app(state.clone())
            .oneshot(authed(
                "POST",
                "/api/v1/jobs",
                Some(r#"{"prompt":"search the web for rust news","schedule":"daily"}"#),
            ))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::CREATED);
        let created = body_json(resp).await;
        let tools = created["granted_scope"]["tools"].as_array().expect("tools");
        assert!(
            tools.iter().any(|t| t == "web_search"),
            "predict_scope applied: {tools:?}"
        );
    }

    #[tokio::test]
    async fn trigger_unknown_job_is_404_known_is_202() {
        let state = make_state().await;
        let resp = app(state.clone())
            .oneshot(authed("POST", "/api/v1/jobs/ghost/trigger", None))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let job = state
            .cron
            .create(CronJob::new(
                "p",
                CronSchedule::Cron("0 0 * * *".into()),
                "s",
            ))
            .await
            .expect("create");
        let resp = app(state)
            .oneshot(authed(
                "POST",
                &format!("/api/v1/jobs/{}/trigger", job.id),
                None,
            ))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
    }

    // ── Runs (criterion 2 surface) ──────────────────────────────────────────

    #[tokio::test]
    async fn runs_endpoints_return_history_and_404s() {
        let state = make_state().await;
        let job = state
            .cron
            .create(CronJob::new(
                "p",
                CronSchedule::Cron("0 0 * * *".into()),
                "s",
            ))
            .await
            .expect("create");
        let run = state
            .runs
            .insert_running(&job.id, chrono::Utc::now())
            .await
            .expect("run");
        state
            .runs
            .finalize(&run.id, RunStatus::Failure, None, Some("boom".into()), None)
            .await
            .expect("finalize");

        let resp = app(state.clone())
            .oneshot(authed(
                "GET",
                &format!("/api/v1/jobs/{}/runs", job.id),
                None,
            ))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        let history = body_json(resp).await;
        assert_eq!(history.as_array().expect("array").len(), 1);
        assert_eq!(history[0]["status"], "failure");
        assert_eq!(history[0]["error"], "boom");

        let resp = app(state.clone())
            .oneshot(authed("GET", &format!("/api/v1/runs/{}", run.id), None))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = app(state)
            .oneshot(authed("GET", "/api/v1/runs/ghost", None))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ── Graph (criteria 4, 10) ──────────────────────────────────────────────

    #[tokio::test]
    async fn graph_returns_all_nodes_and_only_resolvable_edges() {
        let state = make_state().await;
        let parent = state
            .cron
            .create(CronJob::new(
                "parent",
                CronSchedule::Cron("0 0 * * *".into()),
                "s",
            ))
            .await
            .expect("parent");
        let child = state
            .cron
            .create(
                CronJob::new("child", CronSchedule::Cron("0 1 * * *".into()), "s")
                    .with_context_from(vec![parent.id.clone(), "missing-job".into()]),
            )
            .await
            .expect("child");

        let resp = app(state)
            .oneshot(authed("GET", "/api/v1/graph", None))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        let graph = body_json(resp).await;

        let nodes = graph["nodes"].as_array().expect("nodes");
        assert_eq!(nodes.len(), 2, "every job is a node");
        assert!(
            nodes.iter().all(|n| n["latest_run"].is_null()),
            "never-run jobs have null latest_run (neutral grey)"
        );

        let edges = graph["edges"].as_array().expect("edges");
        assert_eq!(edges.len(), 1, "dangling context_from is not an edge");
        assert_eq!(edges[0]["from"], serde_json::json!(parent.id));
        assert_eq!(edges[0]["to"], serde_json::json!(child.id));
    }

    // ── Approvals (criterion 7 surface) ─────────────────────────────────────

    #[tokio::test]
    async fn approvals_list_resolve_roundtrip() {
        let state = make_state().await;
        let (tx, rx) = tokio::sync::oneshot::channel();
        state.approvals.register(
            PendingApproval::new(
                "call-1",
                Some("job-1".into()),
                "terminal",
                serde_json::json!({}),
            ),
            tx,
        );

        let resp = app(state.clone())
            .oneshot(authed("GET", "/api/v1/approvals", None))
            .await
            .expect("response");
        let pending = body_json(resp).await;
        assert_eq!(pending.as_array().expect("array").len(), 1);
        assert_eq!(pending[0]["call_id"], "call-1");

        let resp = app(state.clone())
            .oneshot(authed(
                "POST",
                "/api/v1/approvals/call-1",
                Some(r#"{"approve":false}"#),
            ))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(rx.await, Ok(false), "deny reached the agent");

        let resp = app(state)
            .oneshot(authed(
                "POST",
                "/api/v1/approvals/call-1",
                Some(r#"{"approve":true}"#),
            ))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "already resolved");
    }

    // ── Flows (criteria 5, 6) ───────────────────────────────────────────────

    /// State whose provider returns a canned planner reply, and whose tool
    /// registry contains one Safe and one Dangerous tool.
    async fn make_flow_state(llm_reply: &str) -> WebState {
        struct FakeTool {
            name: &'static str,
            level: talon_core::approval::ApprovalLevel,
        }
        impl talon_core::tools::Tool for FakeTool {
            fn name(&self) -> &str {
                self.name
            }
            fn schema(&self) -> serde_json::Value {
                serde_json::json!({"name": self.name})
            }
            fn approval_level(
                &self,
                _args: &serde_json::Value,
            ) -> talon_core::approval::ApprovalLevel {
                self.level
            }
            fn execute(
                &self,
                _args: serde_json::Value,
                _ctx: talon_core::tools::ToolContext,
            ) -> Pin<Box<dyn Future<Output = talon_core::tools::ToolResult> + Send + '_>>
            {
                Box::pin(async { talon_core::tools::ToolResult::ok("noop") })
            }
        }

        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        let cron = CronStore::new(Arc::clone(&db));
        let tokens = TokenStore::new(Arc::clone(&db));
        let runs = RunStore::new(db);
        let scheduler = Scheduler::new(cron.clone(), Arc::new(NoopRunner));
        let sched = scheduler.handle();
        std::mem::forget(scheduler);
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAP);
        let ctx = Arc::new(
            GatewayContext::new(Arc::new(MockProvider::text(llm_reply, "end_turn")))
                .with_tool(Arc::new(FakeTool {
                    name: "web_search",
                    level: talon_core::approval::ApprovalLevel::Safe,
                }))
                .with_tool(Arc::new(FakeTool {
                    name: "terminal",
                    level: talon_core::approval::ApprovalLevel::Dangerous,
                })),
        );
        WebState::new(
            ctx,
            cron,
            runs,
            sched,
            events,
            ApprovalBroker::new(),
            tokens,
            TOKEN,
        )
        .expect("state")
    }

    const PLAN_REPLY: &str = r#"{"jobs":[
        {"key":"fetch","name":"Fetch","schedule":"0 7 * * *","prompt":"summarize unread email","deliver_to":"local","context_from":[]},
        {"key":"post","name":"Post","schedule":"30 7 * * *","prompt":"post the digest to telegram","deliver_to":"origin","context_from":["fetch"]}
    ]}"#;

    #[tokio::test]
    async fn flows_plan_returns_draft_and_writes_nothing() {
        let state = make_flow_state(PLAN_REPLY).await;
        let resp = app(state.clone())
            .oneshot(authed(
                "POST",
                "/api/v1/flows/plan",
                Some(r#"{"description":"every morning summarize unread email, then post a digest to telegram"}"#),
            ))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        let plan = body_json(resp).await;
        let jobs = plan["jobs"].as_array().expect("jobs");
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[1]["context_from"][0], "fetch");
        assert!(
            jobs[0]["predicted_scope"]["tools"].is_array(),
            "scope predicted per job"
        );
        // Criterion 5: planning persists nothing.
        assert!(state.cron.list().await.expect("list").is_empty());
    }

    #[tokio::test]
    async fn flows_plan_rejects_malformed_llm_reply() {
        let state = make_flow_state("I cannot help with that.").await;
        let resp = app(state.clone())
            .oneshot(authed(
                "POST",
                "/api/v1/flows/plan",
                Some(r#"{"description":"do things"}"#),
            ))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(state.cron.list().await.expect("list").is_empty());
    }

    #[tokio::test]
    async fn flows_commit_strips_dangerous_and_remaps_edges() {
        let state = make_flow_state("unused").await;
        let body = r#"{"jobs":[
            {"key":"fetch","name":"Fetch","schedule":"0 7 * * *","prompt":"p1","context_from":[],
             "granted_scope":{"tools":["web_search","terminal"],"bash_patterns":[]}},
            {"key":"post","name":"Post","schedule":"30 7 * * *","prompt":"p2","context_from":["fetch"],
             "granted_scope":{"tools":["terminal"],"bash_patterns":[]}}
        ]}"#;
        let resp = app(state.clone())
            .oneshot(authed("POST", "/api/v1/flows", Some(body)))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::CREATED);
        let created = body_json(resp).await;
        let created = created["created"].as_array().expect("created");
        assert_eq!(created.len(), 2);

        // Criterion 6: Dangerous tool absent from every persisted scope.
        let jobs = state.cron.list().await.expect("list");
        assert_eq!(jobs.len(), 2);
        for job in &jobs {
            assert!(
                !job.granted_scope.tools.contains(&"terminal".to_string()),
                "Dangerous tool must be stripped: {:?}",
                job.granted_scope
            );
        }
        let fetch = jobs.iter().find(|j| j.prompt == "p1").expect("fetch");
        assert!(
            fetch
                .granted_scope
                .tools
                .contains(&"web_search".to_string()),
            "Safe tool survives"
        );

        // Edges remapped from draft keys to real ids.
        let post = jobs.iter().find(|j| j.prompt == "p2").expect("post");
        assert_eq!(post.context_from, vec![fetch.id.clone()]);
    }

    #[tokio::test]
    async fn flows_commit_rejects_cycle() {
        let state = make_flow_state("unused").await;
        let body = r#"{"jobs":[
            {"key":"a","name":"A","schedule":"0 7 * * *","prompt":"p","context_from":["b"]},
            {"key":"b","name":"B","schedule":"0 8 * * *","prompt":"p","context_from":["a"]}
        ]}"#;
        let resp = app(state.clone())
            .oneshot(authed("POST", "/api/v1/flows", Some(body)))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(state.cron.list().await.expect("list").is_empty());
    }

    // ── Named tokens + roles (criteria 4–6) ─────────────────────────────────

    fn with_token(method: &str, uri: &str, token: &str, body: Option<&str>) -> HttpRequest<Body> {
        let builder = HttpRequest::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json");
        match body {
            Some(b) => builder.body(Body::from(b.to_string())).expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        }
    }

    #[tokio::test]
    async fn viewer_token_reads_but_cannot_mutate() {
        let state = make_state().await;
        let viewer = state
            .tokens
            .create("ro", TokenRole::Viewer)
            .await
            .expect("create");

        let resp = app(state.clone())
            .oneshot(with_token("GET", "/api/v1/jobs", &viewer, None))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK, "viewer can read");

        for (method, uri, body) in [
            (
                "POST",
                "/api/v1/jobs",
                Some(r#"{"prompt":"x","schedule":"daily"}"#),
            ),
            (
                "PATCH",
                "/api/v1/jobs/some-id",
                Some(r#"{"enabled":false}"#),
            ),
            ("DELETE", "/api/v1/jobs/some-id", None),
            (
                "POST",
                "/api/v1/tokens",
                Some(r#"{"name":"x","role":"admin"}"#),
            ),
        ] {
            let resp = app(state.clone())
                .oneshot(with_token(method, uri, &viewer, body))
                .await
                .expect("response");
            assert_eq!(
                resp.status(),
                StatusCode::FORBIDDEN,
                "{method} {uri} must 403 for viewer"
            );
        }
    }

    #[tokio::test]
    async fn named_admin_token_has_full_access_and_me_reports_identity() {
        let state = make_state().await;
        let admin = state
            .tokens
            .create("ops", TokenRole::Admin)
            .await
            .expect("create");

        let resp = app(state.clone())
            .oneshot(with_token(
                "POST",
                "/api/v1/jobs",
                &admin,
                Some(r#"{"prompt":"x","schedule":"daily"}"#),
            ))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::CREATED);

        let resp = app(state)
            .oneshot(with_token("GET", "/api/v1/me", &admin, None))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        let me = body_json(resp).await;
        assert_eq!(me["name"], "ops");
        assert_eq!(me["role"], "admin");
    }

    #[tokio::test]
    async fn legacy_token_is_implicit_admin_via_me() {
        let state = make_state().await;
        let resp = app(state)
            .oneshot(authed("GET", "/api/v1/me", None))
            .await
            .expect("response");
        let me = body_json(resp).await;
        assert_eq!(me["name"], LEGACY_TOKEN_NAME);
        assert_eq!(me["role"], "admin");
    }

    #[tokio::test]
    async fn revoked_token_gets_401() {
        let state = make_state().await;
        let raw = state
            .tokens
            .create("gone", TokenRole::Admin)
            .await
            .expect("create");
        state.tokens.revoke("gone").await.expect("revoke");

        let resp = app(state)
            .oneshot(with_token("GET", "/api/v1/jobs", &raw, None))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn token_crud_roundtrip_shows_raw_once_and_list_never() {
        let state = make_state().await;

        let resp = app(state.clone())
            .oneshot(authed(
                "POST",
                "/api/v1/tokens",
                Some(r#"{"name":"ci","role":"viewer"}"#),
            ))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::CREATED);
        let created = body_json(resp).await;
        let raw = created["token"].as_str().expect("raw token").to_string();
        assert!(raw.starts_with("talon_"));

        // Duplicate name → 409.
        let resp = app(state.clone())
            .oneshot(authed(
                "POST",
                "/api/v1/tokens",
                Some(r#"{"name":"ci","role":"viewer"}"#),
            ))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::CONFLICT);

        // List never contains the raw token.
        let resp = app(state.clone())
            .oneshot(authed("GET", "/api/v1/tokens", None))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        let list = body_json(resp).await;
        assert!(!list.to_string().contains(&raw));

        // Viewer cannot even list tokens.
        let resp = app(state.clone())
            .oneshot(with_token("GET", "/api/v1/tokens", &raw, None))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        // Revoke → 204, then the token stops working.
        let resp = app(state.clone())
            .oneshot(authed("DELETE", "/api/v1/tokens/ci", None))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let resp = app(state)
            .oneshot(with_token("GET", "/api/v1/jobs", &raw, None))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ── Observability surface (criteria 17, 19, 20) ─────────────────────────

    #[tokio::test]
    async fn responses_carry_request_id_header() {
        let state = make_state().await;
        let resp = app(state)
            .oneshot(authed("GET", "/api/v1/jobs", None))
            .await
            .expect("response");
        let id = resp
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .expect("x-request-id present");
        assert_eq!(id.len(), 32, "uuid simple form");
    }

    #[tokio::test]
    async fn metrics_route_is_token_protected_and_renders() {
        let state = make_state().await.with_metrics(Arc::new(|| {
            "talon_runs_total{status=\"success\"} 1\n".to_string()
        }));
        let router = Router::new().merge(metrics_router(state));

        let unauthed = HttpRequest::builder()
            .method("GET")
            .uri("/metrics")
            .body(Body::empty())
            .expect("request");
        let resp = router.clone().oneshot(unauthed).await.expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let resp = router
            .oneshot(authed("GET", "/metrics", None))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        assert!(String::from_utf8_lossy(&body).contains("talon_runs_total"));
    }

    #[tokio::test]
    async fn logs_tail_streams_backlog_with_level_filter() {
        use futures::StreamExt as _;

        let ring = Arc::new(logs::LogRing::new());
        ring.push(logs::LogLine {
            ts: "2026-06-11T00:00:00Z".into(),
            level: "DEBUG".into(),
            target: "t".into(),
            message: "noisy debug line".into(),
        });
        ring.push(logs::LogLine {
            ts: "2026-06-11T00:00:01Z".into(),
            level: "ERROR".into(),
            target: "t".into(),
            message: "important error line".into(),
        });
        let state = make_state().await.with_log_ring(ring);

        let resp = app(state)
            .oneshot(authed("GET", "/api/v1/logs/tail?level=error", None))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let mut body = resp.into_body().into_data_stream();
        let first = tokio::time::timeout(std::time::Duration::from_secs(2), body.next())
            .await
            .expect("first frame in time")
            .expect("stream open")
            .expect("frame ok");
        let text = String::from_utf8_lossy(&first);
        assert!(text.contains("important error line"), "got: {text}");
        assert!(!text.contains("noisy debug line"), "debug filtered: {text}");
    }

    // ── token_eq ────────────────────────────────────────────────────────────

    #[test]
    fn token_eq_matches_only_exact() {
        assert!(token_eq("abc", "abc"));
        assert!(!token_eq("abc", "abd"));
        assert!(!token_eq("abc", "abcd"));
        assert!(!token_eq("", "abc"));
    }
}
