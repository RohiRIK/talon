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
    /// Graph-editor edits (criteria 21–22): replaces the dependency list.
    /// Server re-validates — unknown ids, self-reference, and cycles → 422.
    pub context_from: Option<Vec<String>>,
    /// Retry policy (criterion 28); negative values clamp to 0.
    pub retry_max: Option<i64>,
    /// Error-handler job id (criterion 29); empty string clears it.
    pub on_failure: Option<String>,
}

pub async fn patch_job(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Json(req): Json<PatchJob>,
) -> ApiResult<Json<JobView>> {
    let mut changed = false;

    if let Some(context_from) = req.context_from {
        let jobs = state.cron.list().await.map_err(store_err)?;
        validate_context_edit(&id, &context_from, &jobs)
            .map_err(|reason| err(StatusCode::UNPROCESSABLE_ENTITY, reason))?;
        state
            .cron
            .set_context_from(&id, context_from)
            .await
            .map_err(store_err)?;
        changed = true;
    }

    if req.retry_max.is_some() || req.on_failure.is_some() {
        let current = state
            .cron
            .get(&id)
            .await
            .map_err(store_err)?
            .ok_or_else(|| err(StatusCode::NOT_FOUND, format!("no job {id}")))?;
        let retry_max = req.retry_max.unwrap_or(current.retry_max);
        let on_failure = match req.on_failure {
            None => current.on_failure,
            Some(s) if s.is_empty() => None,
            Some(s) => Some(s),
        };
        // Store-level validation (self-reference, unknown handler) → 422.
        state
            .cron
            .set_reliability(&id, retry_max, on_failure)
            .await
            .map_err(|e| match e {
                talon_memory::MemoryError::Cron(msg) => err(StatusCode::UNPROCESSABLE_ENTITY, msg),
                other => store_err(other),
            })?;
        changed = true;
    }

    if let Some(enabled) = req.enabled {
        state
            .cron
            .set_enabled(&id, enabled)
            .await
            .map_err(store_err)?;
        changed = true;
    }

    if changed {
        let _ = state
            .events
            .send(RunEvent::JobChanged { job_id: id.clone() });
    }
    get_job(State(state), Path(id)).await
}

/// Validate a `context_from` replacement against the whole job graph
/// (criterion 22): every referenced id exists, no self-reference, and the
/// graph with this edit applied stays acyclic (Kahn — leftovers are the
/// cycle, named in the error).
fn validate_context_edit(
    job_id: &str,
    new_deps: &[String],
    jobs: &[talon_memory::CronJob],
) -> Result<(), String> {
    use std::collections::{HashMap, HashSet};

    let ids: HashSet<&str> = jobs.iter().map(|j| j.id.as_str()).collect();
    if !ids.contains(job_id) {
        return Err(format!("no job {job_id}"));
    }
    for dep in new_deps {
        if dep == job_id {
            return Err("a job cannot depend on itself".to_string());
        }
        if !ids.contains(dep.as_str()) {
            return Err(format!("unknown dependency: {dep}"));
        }
    }

    // Graph with the edit applied: edge dep -> job.
    let deps_of = |j: &talon_memory::CronJob| -> Vec<String> {
        if j.id == job_id {
            new_deps.to_vec()
        } else {
            j.context_from.clone()
        }
    };

    let mut indegree: HashMap<&str, usize> = jobs.iter().map(|j| (j.id.as_str(), 0)).collect();
    let mut edges: HashMap<String, Vec<String>> = HashMap::new();
    for job in jobs {
        for dep in deps_of(job) {
            // Dangling deps on OTHER jobs are tolerated (legacy data) — they
            // can never form a cycle.
            if ids.contains(dep.as_str()) {
                *indegree.entry(job.id.as_str()).or_insert(0) += 1;
                edges.entry(dep).or_default().push(job.id.clone());
            }
        }
    }

    let mut ready: Vec<&str> = indegree
        .iter()
        .filter(|(_, deg)| **deg == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut visited = 0usize;
    while let Some(node) = ready.pop() {
        visited += 1;
        if let Some(children) = edges.get(node) {
            for child in children {
                if let Some(deg) = indegree.get_mut(child.as_str()) {
                    *deg -= 1;
                    if *deg == 0 {
                        // SAFETY of lifetime: child borrows from `edges`,
                        // which outlives the loop.
                        if let Some(j) = jobs.iter().find(|j| j.id == *child) {
                            ready.push(j.id.as_str());
                        }
                    }
                }
            }
        }
    }

    if visited < jobs.len() {
        let stuck: Vec<&str> = indegree
            .iter()
            .filter(|(_, deg)| **deg > 0)
            .map(|(id, _)| *id)
            .collect();
        return Err(format!("dependency cycle involving: {}", stuck.join(", ")));
    }
    Ok(())
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
