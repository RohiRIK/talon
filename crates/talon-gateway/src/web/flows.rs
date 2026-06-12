//! AI flow builder endpoints (SPEC Phase 7, criteria 5–6).
//!
//! `POST /api/v1/flows/plan` — natural language → LLM-drafted DAG of cron
//! jobs. **Creates nothing**: the draft (schedules validated, scopes
//! predicted) goes back to the console for the human to edit in the grant box.
//!
//! `POST /api/v1/flows` — commit a user-approved draft. Server-side rules:
//! `Dangerous`-class tools are stripped from every scope (defense in depth —
//! the runtime membrane denies them unattended anyway), drafts with unknown
//! `context_from` keys or cycles are rejected, and jobs are created in
//! dependency order with draft keys remapped to real ids.

use std::collections::{HashMap, HashSet};

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use talon_core::scheduler::RunEvent;
use talon_llm::{ContentBlock, Message};
use talon_memory::{CronJob, GrantedScope, validate_schedule};

use super::WebState;
use super::handlers::{ApiError, ApiResult};

// ── Planner prompt (stable content — keep static for provider-side caching) ──

const PLANNER_PROMPT: &str = r#"You are a scheduling planner for Talon, an AI agent that runs cron jobs.
Each job is one agent run: a prompt executed on a cron schedule. Jobs form a
DAG — a job may list upstream jobs in `context_from`, and it then receives
their last output as context.

Decompose the user's automation description into the smallest sensible set of
jobs (often just one). Use multiple jobs only when steps run on different
schedules or one step consumes another's output.

Respond with **only** a JSON object — no prose, no code fences — in exactly
this shape:

{
  "jobs": [
    {
      "key": "fetch-inbox",
      "name": "Fetch inbox summary",
      "schedule": "0 7 * * *",
      "prompt": "Summarize my unread emails from the last 24 hours.",
      "deliver_to": "local",
      "context_from": []
    },
    {
      "key": "post-digest",
      "name": "Post digest to Telegram",
      "schedule": "30 7 * * *",
      "prompt": "Using the inbox summary you are given, write a short digest and send it to Telegram.",
      "deliver_to": "origin",
      "context_from": ["fetch-inbox"]
    }
  ]
}

Rules:
- `key`: short kebab-case identifier, unique within the plan.
- `schedule`: a standard 5-field cron expression evaluated in the user's timezone.
- `prompt`: a complete, self-contained instruction for the agent run.
- `deliver_to`: "origin", "local", "all", or "platform:chat_id" — use "origin" when unsure.
- `context_from`: keys of upstream jobs whose output this job needs; [] when none.
- Order jobs so that every `context_from` reference points to an earlier job."#;

// ── Types ────────────────────────────────────────────────────────────────────

/// One step in a draft flow. `key` is draft-local; real ids are assigned at
/// commit time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftJob {
    pub key: String,
    pub name: String,
    pub schedule: String,
    pub prompt: String,
    #[serde(default)]
    pub deliver_to: Option<String>,
    #[serde(default)]
    pub tz: Option<String>,
    #[serde(default)]
    pub context_from: Vec<String>,
}

/// What the LLM must return (and what `plan` re-validates).
#[derive(Debug, Deserialize)]
struct LlmPlan {
    jobs: Vec<DraftJob>,
}

/// A draft job enriched with the predicted scope for the grant box.
#[derive(Debug, Serialize)]
pub struct PlannedJob {
    #[serde(flatten)]
    pub job: DraftJob,
    pub predicted_scope: GrantedScope,
}

#[derive(Debug, Deserialize)]
pub struct PlanRequest {
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct PlanResponse {
    pub jobs: Vec<PlannedJob>,
}

/// A commit step: the draft plus the human-edited grant box.
#[derive(Debug, Deserialize)]
pub struct CommitJob {
    #[serde(flatten)]
    pub job: DraftJob,
    #[serde(default)]
    pub granted_scope: GrantedScope,
}

#[derive(Debug, Deserialize)]
pub struct CommitRequest {
    pub jobs: Vec<CommitJob>,
}

#[derive(Debug, Serialize)]
pub struct CommitResponse {
    pub created: Vec<CronJob>,
}

fn unprocessable(msg: impl Into<String>) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(ApiError { error: msg.into() }),
    )
}

// ── Validation shared by plan and commit ────────────────────────────────────

/// Structural validation of a draft: non-empty, unique keys, valid schedules,
/// resolvable references, no cycles. Returns keys in dependency (topological)
/// order so commit can create upstream jobs first.
fn validate_draft(jobs: &[DraftJob]) -> Result<Vec<String>, String> {
    if jobs.is_empty() {
        return Err("plan contains no jobs".to_string());
    }

    let mut keys = HashSet::new();
    for job in jobs {
        if job.key.trim().is_empty() {
            return Err("a job has an empty key".to_string());
        }
        if !keys.insert(job.key.as_str()) {
            return Err(format!("duplicate job key: {}", job.key));
        }
        if job.prompt.trim().is_empty() {
            return Err(format!("job '{}' has an empty prompt", job.key));
        }
        let schedule = talon_tools::parse_schedule(&job.schedule);
        let tz = job.tz.as_deref().unwrap_or("UTC");
        validate_schedule(&schedule, tz)
            .map_err(|e| format!("job '{}' has an invalid schedule: {e}", job.key))?;
    }

    for job in jobs {
        for parent in &job.context_from {
            if !keys.contains(parent.as_str()) {
                return Err(format!(
                    "job '{}' references unknown key '{parent}'",
                    job.key
                ));
            }
            if parent == &job.key {
                return Err(format!("job '{}' references itself", job.key));
            }
        }
    }

    // Kahn topological sort — leftover nodes mean a cycle.
    let mut indegree: HashMap<&str, usize> = jobs
        .iter()
        .map(|j| (j.key.as_str(), j.context_from.len()))
        .collect();
    let mut order: Vec<String> = Vec::with_capacity(jobs.len());
    loop {
        let ready: Vec<&str> = indegree
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(k, _)| *k)
            .collect();
        if ready.is_empty() {
            break;
        }
        for key in ready {
            indegree.remove(key);
            order.push(key.to_string());
            for job in jobs {
                if job.context_from.iter().any(|p| p == key)
                    && let Some(deg) = indegree.get_mut(job.key.as_str())
                {
                    *deg = deg.saturating_sub(1);
                }
            }
        }
    }
    if !indegree.is_empty() {
        let stuck: Vec<&str> = indegree.keys().copied().collect();
        return Err(format!("dependency cycle involving: {}", stuck.join(", ")));
    }
    Ok(order)
}

/// Extract the first JSON object from the model's reply — tolerates stray
/// prose or code fences around it, rejects everything else.
fn extract_json(raw: &str) -> Result<LlmPlan, String> {
    let start = raw.find('{').ok_or("no JSON object in model reply")?;
    let end = raw.rfind('}').ok_or("no JSON object in model reply")?;
    if end < start {
        return Err("malformed JSON in model reply".to_string());
    }
    serde_json::from_str::<LlmPlan>(&raw[start..=end])
        .map_err(|e| format!("model reply is not a valid plan: {e}"))
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// Natural language → validated draft. Writes nothing.
pub async fn plan(
    State(state): State<WebState>,
    Json(req): Json<PlanRequest>,
) -> ApiResult<Json<PlanResponse>> {
    if req.description.trim().is_empty() {
        return Err(unprocessable("description is empty"));
    }

    let messages = [
        Message::system(PLANNER_PROMPT),
        Message::user(req.description.clone()),
    ];
    let response = state
        .ctx
        .provider
        .complete(&messages, &[])
        .await
        .map_err(|e| unprocessable(format!("planner LLM call failed: {e}")))?;

    let text: String = response
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            ContentBlock::ToolUse { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let plan = extract_json(&text).map_err(unprocessable)?;
    validate_draft(&plan.jobs).map_err(unprocessable)?;

    let jobs = plan
        .jobs
        .into_iter()
        .map(|job| {
            let predicted_scope = talon_tools::predict_scope(&job.prompt);
            PlannedJob {
                job,
                predicted_scope,
            }
        })
        .collect();
    Ok(Json(PlanResponse { jobs }))
}

/// Commit an approved draft: strip Dangerous tools, create in dependency
/// order, remap draft keys to real job ids.
pub async fn commit(
    State(state): State<WebState>,
    Json(req): Json<CommitRequest>,
) -> ApiResult<(StatusCode, Json<CommitResponse>)> {
    let drafts: Vec<DraftJob> = req.jobs.iter().map(|j| j.job.clone()).collect();
    let order = validate_draft(&drafts).map_err(unprocessable)?;

    // Tools whose baseline approval (empty args) is Dangerous never enter an
    // unattended scope — even if the client (or the LLM) granted them.
    let dangerous: HashSet<String> = state
        .ctx
        .tools
        .iter()
        .filter(|t| {
            t.approval_level(&serde_json::Value::Null)
                == talon_core::approval::ApprovalLevel::Dangerous
        })
        .map(|t| t.name().to_string())
        .collect();

    let by_key: HashMap<&str, &CommitJob> =
        req.jobs.iter().map(|j| (j.job.key.as_str(), j)).collect();

    let mut key_to_id: HashMap<String, String> = HashMap::new();
    let mut created = Vec::with_capacity(order.len());
    for key in &order {
        let commit_job = by_key
            .get(key.as_str())
            .ok_or_else(|| unprocessable(format!("internal: lost job '{key}'")))?;
        let draft = &commit_job.job;

        let mut scope = commit_job.granted_scope.clone();
        scope.tools.retain(|t| !dangerous.contains(t));

        let mut job = CronJob::new(
            draft.prompt.clone(),
            talon_tools::parse_schedule(&draft.schedule),
            String::new(),
        );
        job.session_id = format!("cron-{}", job.id);
        job = job.with_name(draft.name.clone()).with_granted_scope(scope);
        if let Some(target) = draft.deliver_to.clone().filter(|s| !s.trim().is_empty()) {
            job = job.with_deliver_to(target);
        }
        if let Some(tz) = draft.tz.clone().filter(|s| !s.trim().is_empty()) {
            job = job.with_tz(tz);
        }
        let parents: Vec<String> = draft
            .context_from
            .iter()
            .filter_map(|k| key_to_id.get(k).cloned())
            .collect();
        if !parents.is_empty() {
            job = job.with_context_from(parents);
        }

        let stored = state
            .cron
            .create(job)
            .await
            .map_err(|e| unprocessable(format!("failed to create job '{key}': {e}")))?;
        let _ = state.events.send(RunEvent::JobChanged {
            job_id: stored.id.clone(),
        });
        key_to_id.insert(key.clone(), stored.id.clone());
        created.push(stored);
    }

    Ok((StatusCode::CREATED, Json(CommitResponse { created })))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn draft(key: &str, parents: &[&str]) -> DraftJob {
        DraftJob {
            key: key.to_string(),
            name: key.to_string(),
            schedule: "0 9 * * *".to_string(),
            prompt: format!("do {key}"),
            deliver_to: None,
            tz: None,
            context_from: parents.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn validate_orders_dependencies_first() {
        let jobs = vec![draft("c", &["b"]), draft("b", &["a"]), draft("a", &[])];
        let order = validate_draft(&jobs).expect("valid");
        assert_eq!(order, vec!["a", "b", "c"]);
    }

    #[test]
    fn validate_rejects_cycle_unknown_ref_dupes_and_empty() {
        assert!(validate_draft(&[]).is_err(), "empty plan");

        let cycle = vec![draft("a", &["b"]), draft("b", &["a"])];
        assert!(
            validate_draft(&cycle).expect_err("cycle").contains("cycle"),
            "cycle detected"
        );

        let unknown = vec![draft("a", &["ghost"])];
        assert!(validate_draft(&unknown).is_err(), "unknown ref");

        let dupes = vec![draft("a", &[]), draft("a", &[])];
        assert!(validate_draft(&dupes).is_err(), "duplicate keys");

        let mut bad_cron = vec![draft("a", &[])];
        bad_cron[0].schedule = "totally not cron".to_string();
        assert!(validate_draft(&bad_cron).is_err(), "bad schedule");
    }

    #[test]
    fn extract_json_tolerates_fences_and_prose() {
        let raw = "Here is your plan:\n```json\n{\"jobs\":[{\"key\":\"a\",\"name\":\"A\",\"schedule\":\"0 9 * * *\",\"prompt\":\"p\"}]}\n```";
        let plan = extract_json(raw).expect("parse");
        assert_eq!(plan.jobs.len(), 1);
        assert_eq!(plan.jobs[0].key, "a");

        assert!(extract_json("no json here at all").is_err());
    }
}
