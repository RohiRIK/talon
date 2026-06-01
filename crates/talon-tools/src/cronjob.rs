//! `CronJobTool` (SPEC §4.4 / S3) — the natural-language surface for scheduling.
//! Create, list, and delete cron jobs in the [`CronStore`].
//!
//! **Approval model.** Mutating actions (`create`, `delete`) are
//! [`ApprovalLevel::NeedsApproval`]: the human confirms *here*, while present, and
//! that confirmation **is** the §4.4 wizard gate. `list` is read-only and `Safe`.
//!
//! **Creation-time scope wizard.** On `create`, if the caller supplies no
//! `granted_scope`, [`predict_scope`] runs a deterministic dry-analysis over the
//! prompt and predicts the capability allowlist the job will need (tool names +
//! concrete Bash command patterns). Predictions are intentionally conservative —
//! only `Safe`/`NeedsApproval` tools are ever auto-granted, never `Dangerous`
//! ones — and a caller can always override the predicted set explicitly.

use std::{future::Future, pin::Pin, sync::Arc};

use serde_json::{Value, json};
use talon_core::{
    approval::ApprovalLevel,
    tools::{Tool, ToolContext, ToolResult},
};
use talon_memory::{CronJob, CronSchedule, CronStore, GrantedScope};

/// Schedule the agent to run a prompt on a cron cadence.
pub struct CronJobTool {
    store: Arc<CronStore>,
}

impl CronJobTool {
    pub fn new(store: Arc<CronStore>) -> Self {
        Self { store }
    }
}

impl Tool for CronJobTool {
    fn name(&self) -> &str {
        "cron_job"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "cron_job",
            "description": "Create, list, or delete scheduled (cron) jobs. A job runs a prompt as the agent on a recurring schedule and delivers its output to a target. On create, the job's tool/Bash capability scope is predicted and must be confirmed (NeedsApproval).",
            "input_schema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["create", "list", "delete"],
                        "description": "What to do."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "create: the instruction the agent runs each fire."
                    },
                    "schedule": {
                        "type": "string",
                        "description": "create: a 5-field cron expression (\"0 9 * * *\") or a friendly form (\"daily\", \"every 2h\", \"30m\")."
                    },
                    "name": {
                        "type": "string",
                        "description": "create: optional human label for the job."
                    },
                    "deliver_to": {
                        "type": "string",
                        "description": "create: where output goes — origin / local / all / platform:chat_id:thread_id (default: origin)."
                    },
                    "tz": {
                        "type": "string",
                        "description": "create: IANA timezone for the schedule (default: UTC)."
                    },
                    "repeat": {
                        "type": "integer",
                        "description": "create: number of times to run (omit for infinite, 1 for one-shot)."
                    },
                    "session_id": {
                        "type": "string",
                        "description": "create: stable session for LTM continuity (default: derived per job)."
                    },
                    "context_from": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "create: upstream job IDs whose last output is injected (job DAG)."
                    },
                    "granted_scope": {
                        "type": "object",
                        "description": "create: explicit capability allowlist; if omitted it is predicted from the prompt.",
                        "properties": {
                            "tools": { "type": "array", "items": { "type": "string" } },
                            "bash_patterns": { "type": "array", "items": { "type": "string" } }
                        }
                    },
                    "id": {
                        "type": "string",
                        "description": "delete: the job id to remove."
                    }
                },
                "required": ["action"]
            }
        })
    }

    fn approval_level(&self, args: &Value) -> ApprovalLevel {
        match args["action"].as_str() {
            // Read-only listing carries no side effect.
            Some("list") => ApprovalLevel::Safe,
            // create/delete (and any unknown action that reaches execute) mutate
            // state — the human confirms while present. This is the §4.4 gate.
            _ => ApprovalLevel::NeedsApproval,
        }
    }

    fn execute(
        &self,
        args: Value,
        _ctx: ToolContext,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let store = Arc::clone(&self.store);
        Box::pin(async move {
            match args["action"].as_str() {
                Some("create") => create_job(&store, &args).await,
                Some("list") => list_jobs(&store).await,
                Some("delete") => delete_job(&store, &args).await,
                Some(other) => ToolResult::err(format!(
                    "unknown action '{other}' — expected create, list, or delete"
                )),
                None => ToolResult::err("missing required argument: action"),
            }
        })
    }
}

async fn create_job(store: &CronStore, args: &Value) -> ToolResult {
    let prompt = match args["prompt"].as_str() {
        Some(p) if !p.trim().is_empty() => p.to_string(),
        _ => return ToolResult::err("create requires a non-empty 'prompt'"),
    };
    let schedule_raw = match args["schedule"].as_str() {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => return ToolResult::err("create requires a 'schedule' (cron expr or friendly form)"),
    };
    let schedule = parse_schedule(&schedule_raw);

    // A new job whose session defaults to a stable per-job id for LTM continuity.
    let mut job = CronJob::new(prompt.clone(), schedule, String::new());
    job.session_id = match args["session_id"].as_str() {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => format!("cron-{}", job.id),
    };

    if let Some(name) = args["name"].as_str().filter(|s| !s.trim().is_empty()) {
        job = job.with_name(name);
    }
    if let Some(target) = args["deliver_to"].as_str().filter(|s| !s.trim().is_empty()) {
        job = job.with_deliver_to(target);
    }
    if let Some(tz) = args["tz"].as_str().filter(|s| !s.trim().is_empty()) {
        job = job.with_tz(tz);
    }
    if let Some(repeat) = args["repeat"].as_i64() {
        job = job.with_repeat(Some(repeat));
    }
    if let Some(ids) = args["context_from"].as_array() {
        let ids: Vec<String> = ids
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        job = job.with_context_from(ids);
    }

    // Scope: explicit grant wins; otherwise predict from the prompt (the wizard).
    let scope =
        parse_granted_scope(&args["granted_scope"]).unwrap_or_else(|| predict_scope(&prompt));
    job = job.with_granted_scope(scope.clone());

    match store.create(job).await {
        Ok(created) => {
            let next = created.next_run.as_deref().unwrap_or("(one-shot/none)");
            ToolResult::ok(format!(
                "Created cron job '{}' (id {}).\n  schedule: {}\n  next run: {}\n  deliver to: {}\n  granted scope: tools=[{}] bash=[{}]",
                created.name.as_deref().unwrap_or("(unnamed)"),
                created.id,
                schedule_raw,
                next,
                created.deliver_to,
                scope.tools.join(", "),
                scope.bash_patterns.join(", "),
            ))
        }
        Err(e) => ToolResult::err(format!("failed to create cron job: {e}")),
    }
}

async fn list_jobs(store: &CronStore) -> ToolResult {
    match store.list().await {
        Ok(jobs) if jobs.is_empty() => ToolResult::ok("No cron jobs scheduled."),
        Ok(jobs) => {
            let lines: Vec<String> = jobs
                .iter()
                .map(|j| {
                    let mark = if j.enabled { "●" } else { "○" };
                    let next = j.next_run.as_deref().unwrap_or("—");
                    format!(
                        "{mark} {} [{}] next={} → {}",
                        j.name.as_deref().unwrap_or(&j.id),
                        j.id,
                        next,
                        j.deliver_to,
                    )
                })
                .collect();
            ToolResult::ok(lines.join("\n"))
        }
        Err(e) => ToolResult::err(format!("failed to list cron jobs: {e}")),
    }
}

async fn delete_job(store: &CronStore, args: &Value) -> ToolResult {
    let id = match args["id"].as_str() {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => return ToolResult::err("delete requires a job 'id'"),
    };
    match store.delete(&id).await {
        Ok(true) => ToolResult::ok(format!("Deleted cron job {id}.")),
        Ok(false) => ToolResult::err(format!("no cron job with id {id}")),
        Err(e) => ToolResult::err(format!("failed to delete cron job: {e}")),
    }
}

/// Decide whether a schedule string is a raw cron expression or a friendly form.
/// A cron expression is 5 or 6 whitespace fields built only from cron tokens
/// (`0-9 * / , -`). Anything else is treated as `Human` and lowered downstream.
fn parse_schedule(raw: &str) -> CronSchedule {
    let trimmed = raw.trim();
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    let is_cron = matches!(fields.len(), 5 | 6)
        && fields.iter().all(|f| {
            f.chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '*' | '/' | ',' | '-'))
        });
    if is_cron {
        CronSchedule::Cron(trimmed.to_string())
    } else {
        CronSchedule::Human(trimmed.to_string())
    }
}

/// Parse an explicit `granted_scope` JSON object, if present and well-formed.
fn parse_granted_scope(value: &Value) -> Option<GrantedScope> {
    if !value.is_object() {
        return None;
    }
    let collect = |key: &str| -> Vec<String> {
        value[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(GrantedScope {
        tools: collect("tools"),
        bash_patterns: collect("bash_patterns"),
    })
}

/// The creation-wizard dry-analysis (deterministic baseline): predict the
/// capability scope a job needs from its prompt. Conservative by construction —
/// it only ever grants `Safe`/`NeedsApproval` tools, never `Dangerous` ones, and
/// Bash is granted as concrete command patterns, never a blanket "may run Bash".
///
/// An LLM-driven prediction pass can refine this later; this heuristic is the
/// safe floor that needs no network and is fully testable.
pub fn predict_scope(prompt: &str) -> GrantedScope {
    let lower = prompt.to_lowercase();
    let mut tools: Vec<String> = Vec::new();
    let mut bash_patterns: Vec<String> = Vec::new();

    let grant = |needle_hit: bool, tool: &str, tools: &mut Vec<String>| {
        if needle_hit && !tools.iter().any(|t| t == tool) {
            tools.push(tool.to_string());
        }
    };

    let any = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));

    grant(
        any(&["read", "summarize", "file", "notes", "open"]),
        "read_file",
        &mut tools,
    );
    grant(
        any(&["search", "look up", "google", "find online", "web"]),
        "web_search",
        &mut tools,
    );
    grant(
        any(&["fetch", "extract", "url", "webpage", "article", "http"]),
        "web_extract",
        &mut tools,
    );
    grant(
        any(&[
            "post", "send", "notify", "message", "tell", "telegram", "email",
        ]),
        "send_message",
        &mut tools,
    );
    grant(
        any(&["recall", "remember", "past conversation", "history"]),
        "session_search",
        &mut tools,
    );

    // Bash: only specific, well-known read-only commands are predicted, as
    // concrete patterns. A job granted `git pull` can never later `rm -rf`.
    for (needle, pattern) in [
        ("git pull", "git pull"),
        ("git fetch", "git fetch"),
        ("git status", "git status"),
    ] {
        if lower.contains(needle) {
            if !tools.iter().any(|t| t == "terminal") {
                tools.push("terminal".to_string());
            }
            if !bash_patterns.iter().any(|p| p == pattern) {
                bash_patterns.push(pattern.to_string());
            }
        }
    }

    GrantedScope {
        tools,
        bash_patterns,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;

    use talon_memory::Database;

    use super::*;

    async fn make_tool() -> CronJobTool {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        CronJobTool::new(Arc::new(CronStore::new(db)))
    }

    // ── metadata ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn tool_name_is_cron_job() {
        assert_eq!(make_tool().await.name(), "cron_job");
    }

    #[tokio::test]
    async fn schema_requires_action() {
        let tool = make_tool().await;
        let s = tool.schema();
        assert_eq!(s["name"], "cron_job");
        assert_eq!(s["input_schema"]["required"][0], "action");
    }

    #[tokio::test]
    async fn create_and_delete_need_approval_list_is_safe() {
        let tool = make_tool().await;
        assert_eq!(
            tool.approval_level(&json!({"action": "create"})),
            ApprovalLevel::NeedsApproval
        );
        assert_eq!(
            tool.approval_level(&json!({"action": "delete"})),
            ApprovalLevel::NeedsApproval
        );
        assert_eq!(
            tool.approval_level(&json!({"action": "list"})),
            ApprovalLevel::Safe
        );
    }

    // ── create / list / delete ──────────────────────────────────────────────

    #[tokio::test]
    async fn create_persists_a_job() {
        let tool = make_tool().await;
        let result = tool
            .execute(
                json!({
                    "action": "create",
                    "prompt": "summarize my notes",
                    "schedule": "0 9 * * *",
                    "name": "morning-digest"
                }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert!(result.content.contains("morning-digest"));
        assert!(result.content.contains("granted scope"));
    }

    #[tokio::test]
    async fn create_missing_prompt_errors() {
        let tool = make_tool().await;
        let result = tool
            .execute(
                json!({"action": "create", "schedule": "daily"}),
                ToolContext::default(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("prompt"));
    }

    #[tokio::test]
    async fn create_missing_schedule_errors() {
        let tool = make_tool().await;
        let result = tool
            .execute(
                json!({"action": "create", "prompt": "do a thing"}),
                ToolContext::default(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("schedule"));
    }

    #[tokio::test]
    async fn list_empty_then_populated() {
        let tool = make_tool().await;
        let empty = tool
            .execute(json!({"action": "list"}), ToolContext::default())
            .await;
        assert!(!empty.is_error);
        assert!(empty.content.contains("No cron jobs"));

        tool.execute(
            json!({"action": "create", "prompt": "ping", "schedule": "every 2h", "name": "pinger"}),
            ToolContext::default(),
        )
        .await;

        let listed = tool
            .execute(json!({"action": "list"}), ToolContext::default())
            .await;
        assert!(!listed.is_error);
        assert!(listed.content.contains("pinger"));
        assert!(listed.content.contains("●"));
    }

    #[tokio::test]
    async fn delete_removes_a_job() {
        let tool = make_tool().await;
        let created = tool
            .execute(
                json!({"action": "create", "prompt": "x", "schedule": "daily", "name": "tmp"}),
                ToolContext::default(),
            )
            .await;
        // Extract the id from the create message.
        let id = created
            .content
            .split("id ")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("id in message")
            .to_string();

        let deleted = tool
            .execute(
                json!({"action": "delete", "id": id}),
                ToolContext::default(),
            )
            .await;
        assert!(!deleted.is_error, "unexpected: {}", deleted.content);
        assert!(deleted.content.contains("Deleted"));
    }

    #[tokio::test]
    async fn delete_unknown_id_errors() {
        let tool = make_tool().await;
        let result = tool
            .execute(
                json!({"action": "delete", "id": "does-not-exist"}),
                ToolContext::default(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("no cron job"));
    }

    #[tokio::test]
    async fn unknown_action_errors() {
        let tool = make_tool().await;
        let result = tool
            .execute(json!({"action": "frobnicate"}), ToolContext::default())
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("unknown action"));
    }

    // ── parse_schedule ──────────────────────────────────────────────────────

    #[test]
    fn parse_schedule_detects_cron_expr() {
        assert_eq!(
            parse_schedule("0 9 * * *"),
            CronSchedule::Cron("0 9 * * *".to_string())
        );
        assert_eq!(
            parse_schedule("*/5 * * * *"),
            CronSchedule::Cron("*/5 * * * *".to_string())
        );
    }

    #[test]
    fn parse_schedule_treats_friendly_as_human() {
        assert_eq!(
            parse_schedule("every 2h"),
            CronSchedule::Human("every 2h".to_string())
        );
        assert_eq!(
            parse_schedule("daily"),
            CronSchedule::Human("daily".to_string())
        );
        assert_eq!(
            parse_schedule("30m"),
            CronSchedule::Human("30m".to_string())
        );
    }

    // ── predict_scope (the wizard dry-analysis) ─────────────────────────────

    #[test]
    fn predict_scope_email_summary_grants_read_and_send() {
        let scope = predict_scope("every morning summarize my unread emails and post to telegram");
        assert!(scope.tools.contains(&"read_file".to_string()));
        assert!(scope.tools.contains(&"send_message".to_string()));
        // Never a blanket Bash grant for a prose prompt.
        assert!(scope.bash_patterns.is_empty());
    }

    #[test]
    fn predict_scope_web_research_grants_search() {
        let scope = predict_scope("search the web for rust news and extract the top article");
        assert!(scope.tools.contains(&"web_search".to_string()));
        assert!(scope.tools.contains(&"web_extract".to_string()));
    }

    #[test]
    fn predict_scope_git_pull_grants_concrete_bash_pattern_only() {
        let scope = predict_scope("run git pull in my repo every hour");
        assert!(scope.tools.contains(&"terminal".to_string()));
        assert_eq!(scope.bash_patterns, vec!["git pull".to_string()]);
        // The blanket danger is exactly what we must not grant.
        assert!(!scope.bash_patterns.iter().any(|p| p.contains("rm")));
    }

    #[test]
    fn predict_scope_unmatched_prompt_grants_nothing() {
        let scope = predict_scope("xyzzy plugh frobozz");
        assert!(scope.tools.is_empty());
        assert!(scope.bash_patterns.is_empty());
    }

    #[test]
    fn parse_granted_scope_reads_explicit_object() {
        let v = json!({"tools": ["read_file"], "bash_patterns": ["ls"]});
        let scope = parse_granted_scope(&v).expect("parsed");
        assert_eq!(scope.tools, vec!["read_file".to_string()]);
        assert_eq!(scope.bash_patterns, vec!["ls".to_string()]);
    }

    #[test]
    fn parse_granted_scope_rejects_non_object() {
        assert!(parse_granted_scope(&Value::Null).is_none());
    }
}
