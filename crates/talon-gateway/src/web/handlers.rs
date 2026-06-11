//! `/api/v1` request handlers — thin wrappers over `CronStore`, `RunStore`,
//! and the `SchedulerHandle`. All business rules live in the stores; handlers
//! only translate HTTP ⇄ store calls and map `MemoryError` onto status codes.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use talon_core::scheduler::RunEvent;
use talon_memory::{CronJob, CronRun, GrantedScope, MemoryError};

use super::WebState;

/// JSON error body shared by every handler.
#[derive(Debug, Serialize)]
pub struct ApiError {
    pub error: String,
}

pub type ApiResult<T> = Result<T, (StatusCode, Json<ApiError>)>;

fn err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ApiError>) {
    (status, Json(ApiError { error: msg.into() }))
}

/// Map store errors onto HTTP: missing rows → 404, schedule/timezone problems
/// (user input) → 422, anything else → 500.
fn store_err(e: MemoryError) -> (StatusCode, Json<ApiError>) {
    match &e {
        MemoryError::NotFound(_) => err(StatusCode::NOT_FOUND, e.to_string()),
        MemoryError::Cron(_) => err(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
        _ => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ── Jobs ─────────────────────────────────────────────────────────────────────

/// A job plus its most recent execution attempt (drives dashboard + graph).
#[derive(Debug, Serialize)]
pub struct JobView {
    #[serde(flatten)]
    pub job: CronJob,
    pub latest_run: Option<CronRun>,
}

async fn job_views(state: &WebState) -> Result<Vec<JobView>, MemoryError> {
    let jobs = state.cron.list().await?;
    let mut latest: HashMap<String, CronRun> = state
        .runs
        .latest_per_job()
        .await?
        .into_iter()
        .map(|r| (r.job_id.clone(), r))
        .collect();
    Ok(jobs
        .into_iter()
        .map(|job| {
            let latest_run = latest.remove(&job.id);
            JobView { job, latest_run }
        })
        .collect())
}

pub async fn list_jobs(State(state): State<WebState>) -> ApiResult<Json<Vec<JobView>>> {
    job_views(&state).await.map(Json).map_err(store_err)
}

#[derive(Debug, Deserialize)]
pub struct CreateJob {
    pub prompt: String,
    /// 5-field cron ("0 9 * * *"), friendly form ("daily", "every 2h"),
    /// or "@<unix-seconds>" for a one-shot.
    pub schedule: String,
    pub name: Option<String>,
    pub deliver_to: Option<String>,
    pub tz: Option<String>,
    pub repeat: Option<i64>,
    pub session_id: Option<String>,
    #[serde(default)]
    pub context_from: Vec<String>,
    /// Pre-authorized scope (the grant box). Omitted → predicted from the
    /// prompt, same as the CLI wizard. `Dangerous` tools are never honored
    /// unattended regardless of what is granted here (runtime membrane rule).
    pub granted_scope: Option<GrantedScope>,
}

pub async fn create_job(
    State(state): State<WebState>,
    Json(req): Json<CreateJob>,
) -> ApiResult<(StatusCode, Json<JobView>)> {
    if req.prompt.trim().is_empty() {
        return Err(err(StatusCode::UNPROCESSABLE_ENTITY, "prompt is empty"));
    }

    let mut job = CronJob::new(
        req.prompt.clone(),
        talon_tools::parse_schedule(&req.schedule),
        String::new(),
    );
    job.session_id = req
        .session_id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("cron-{}", job.id));
    if let Some(name) = req.name.filter(|s| !s.trim().is_empty()) {
        job = job.with_name(name);
    }
    if let Some(target) = req.deliver_to.filter(|s| !s.trim().is_empty()) {
        job = job.with_deliver_to(target);
    }
    if let Some(tz) = req.tz.filter(|s| !s.trim().is_empty()) {
        job = job.with_tz(tz);
    }
    if let Some(repeat) = req.repeat {
        job = job.with_repeat(Some(repeat));
    }
    if !req.context_from.is_empty() {
        job = job.with_context_from(req.context_from);
    }
    let scope = req
        .granted_scope
        .unwrap_or_else(|| talon_tools::predict_scope(&req.prompt));
    job = job.with_granted_scope(scope);

    let created = state.cron.create(job).await.map_err(store_err)?;
    let _ = state.events.send(RunEvent::JobChanged {
        job_id: created.id.clone(),
    });
    Ok((
        StatusCode::CREATED,
        Json(JobView {
            job: created,
            latest_run: None,
        }),
    ))
}

pub async fn get_job(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> ApiResult<Json<JobView>> {
    let job = state
        .cron
        .get(&id)
        .await
        .map_err(store_err)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("no job {id}")))?;
    let latest_run = state
        .runs
        .list_for_job(&id, 1)
        .await
        .map_err(store_err)?
        .into_iter()
        .next();
    Ok(Json(JobView { job, latest_run }))
}

#[derive(Debug, Deserialize)]
pub struct PatchJob {
    pub enabled: Option<bool>,
}

pub async fn patch_job(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Json(req): Json<PatchJob>,
) -> ApiResult<Json<JobView>> {
    if let Some(enabled) = req.enabled {
        state
            .cron
            .set_enabled(&id, enabled)
            .await
            .map_err(store_err)?;
        let _ = state
            .events
            .send(RunEvent::JobChanged { job_id: id.clone() });
    }
    get_job(State(state), Path(id)).await
}

pub async fn delete_job(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let removed = state.cron.delete(&id).await.map_err(store_err)?;
    if !removed {
        return Err(err(StatusCode::NOT_FOUND, format!("no job {id}")));
    }
    let _ = state.events.send(RunEvent::JobChanged { job_id: id });
    Ok(StatusCode::NO_CONTENT)
}

/// Jenkins "Build Now": queue an immediate manual run. The schedule is not
/// advanced — only `cron_runs` records the attempt.
pub async fn trigger_job(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    state
        .cron
        .get(&id)
        .await
        .map_err(store_err)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("no job {id}")))?;

    if !state.sched.trigger(&id).await {
        return Err(err(
            StatusCode::SERVICE_UNAVAILABLE,
            "scheduler is not running",
        ));
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "status": "queued", "job_id": id })),
    ))
}

// ── Runs ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RunsQuery {
    pub limit: Option<i64>,
}

pub async fn list_runs(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(q): Query<RunsQuery>,
) -> ApiResult<Json<Vec<CronRun>>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    state
        .runs
        .list_for_job(&id, limit)
        .await
        .map(Json)
        .map_err(store_err)
}

pub async fn get_run(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> ApiResult<Json<CronRun>> {
    state
        .runs
        .get(&id)
        .await
        .map_err(store_err)?
        .map(Json)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("no run {id}")))
}

// ── Graph ────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct GraphEdge {
    /// Upstream job (the `context_from` source).
    pub from: String,
    /// Downstream job (the one that consumes the output).
    pub to: String,
}

#[derive(Debug, Serialize)]
pub struct Graph {
    pub nodes: Vec<JobView>,
    pub edges: Vec<GraphEdge>,
}

/// Every job is a node (orphans and cycle members included by construction);
/// an edge exists only where both ends are present in the job set.
pub async fn graph(State(state): State<WebState>) -> ApiResult<Json<Graph>> {
    let nodes = job_views(&state).await.map_err(store_err)?;
    let ids: std::collections::HashSet<&str> = nodes.iter().map(|n| n.job.id.as_str()).collect();
    let edges = nodes
        .iter()
        .flat_map(|n| {
            n.job
                .context_from
                .iter()
                .filter(|p| ids.contains(p.as_str()))
                .map(|p| GraphEdge {
                    from: p.clone(),
                    to: n.job.id.clone(),
                })
        })
        .collect();
    Ok(Json(Graph { nodes, edges }))
}

// ── Approvals ────────────────────────────────────────────────────────────────

pub async fn list_approvals(
    State(state): State<WebState>,
) -> Json<Vec<super::approvals::PendingApproval>> {
    Json(state.approvals.pending())
}

#[derive(Debug, Deserialize)]
pub struct ResolveApproval {
    pub approve: bool,
}

pub async fn resolve_approval(
    State(state): State<WebState>,
    Path(call_id): Path<String>,
    Json(req): Json<ResolveApproval>,
) -> ApiResult<Json<serde_json::Value>> {
    if !state.approvals.resolve(&call_id, req.approve) {
        return Err(err(
            StatusCode::NOT_FOUND,
            format!("no pending approval {call_id}"),
        ));
    }
    let _ = state.events.send(RunEvent::ApprovalResolved {
        call_id: call_id.clone(),
        approved: req.approve,
    });
    Ok(Json(
        serde_json::json!({ "call_id": call_id, "approved": req.approve }),
    ))
}
