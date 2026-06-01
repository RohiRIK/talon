//! `talon cron` — a first-class CLI subcommand (peer to `talon memory` /
//! `talon cache`) that renders scheduled jobs as a **tree** (SPEC §4.1 S6).
//!
//! The job DAG is the natural hierarchy: root jobs at the top, `context_from`
//! children nested beneath the job that feeds them. Each node shows its status
//! glyph (`●` enabled / `○` disabled), name, humanized next run ("in 2h"),
//! schedule, and `deliver_to` target. The renderer is a pure function over a
//! job list + a fixed `now`, so it is unit-testable without wall-clock or DB.

use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::Subcommand;
use talon_memory::{CronJob, CronSchedule, CronStore, Database};

#[derive(Subcommand)]
pub enum CronAction {
    /// Schedule a new job (capability scope is predicted from the prompt).
    Create {
        /// The instruction the agent runs each fire.
        prompt: String,
        /// A 5-field cron expr ("0 9 * * *") or a friendly form ("daily", "every 2h").
        schedule: String,
        /// Optional human label for the job.
        #[arg(long)]
        name: Option<String>,
        /// Where output goes: origin / local / all / platform:chat_id:thread_id.
        #[arg(long)]
        deliver_to: Option<String>,
        /// IANA timezone for the schedule (default: UTC).
        #[arg(long)]
        tz: Option<String>,
        /// Number of times to run (omit for infinite, 1 for one-shot).
        #[arg(long)]
        repeat: Option<i64>,
        /// Stable session id for LTM continuity (default: derived per job).
        #[arg(long)]
        session_id: Option<String>,
        /// Upstream job ids whose last output is injected (repeatable).
        #[arg(long = "context-from")]
        context_from: Vec<String>,
    },
    /// Render all scheduled jobs as a tree (default).
    List,
    /// Enable a job by id.
    Enable {
        /// The job id.
        id: String,
    },
    /// Disable a job by id (keeps it; stops it firing).
    Disable {
        /// The job id.
        id: String,
    },
    /// Delete a job by id.
    Rm {
        /// The job id.
        id: String,
    },
}

pub async fn run(action: CronAction) -> Result<()> {
    let db_path = crate::default_db_path();
    let db = Database::open(db_path.to_str().unwrap_or(":memory:"))
        .map_err(|e| anyhow::anyhow!("failed to open database: {e}"))?;
    db.init_schema()
        .await
        .map_err(|e| anyhow::anyhow!("failed to run migrations: {e}"))?;
    let store = CronStore::new(std::sync::Arc::new(db));
    run_with(&store, action).await
}

async fn run_with(store: &CronStore, action: CronAction) -> Result<()> {
    match action {
        CronAction::Create {
            prompt,
            schedule,
            name,
            deliver_to,
            tz,
            repeat,
            session_id,
            context_from,
        } => {
            let mut job = CronJob::new(
                prompt.clone(),
                talon_tools::parse_schedule(&schedule),
                String::new(),
            );
            job.session_id = session_id
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| format!("cron-{}", job.id));
            if let Some(name) = name.filter(|s| !s.trim().is_empty()) {
                job = job.with_name(name);
            }
            if let Some(target) = deliver_to.filter(|s| !s.trim().is_empty()) {
                job = job.with_deliver_to(target);
            }
            if let Some(tz) = tz.filter(|s| !s.trim().is_empty()) {
                job = job.with_tz(tz);
            }
            if let Some(repeat) = repeat {
                job = job.with_repeat(Some(repeat));
            }
            if !context_from.is_empty() {
                job = job.with_context_from(context_from);
            }
            job = job.with_granted_scope(talon_tools::predict_scope(&prompt));

            let created = store
                .create(job)
                .await
                .map_err(|e| anyhow::anyhow!("failed to create cron job: {e}"))?;
            let next = created.next_run.as_deref().unwrap_or("(one-shot/none)");
            println!(
                "created {} ({})  next: {}  → {}",
                created.name.as_deref().unwrap_or("(unnamed)"),
                created.id,
                next,
                created.deliver_to,
            );
        }
        CronAction::List => {
            let jobs = store
                .list()
                .await
                .map_err(|e| anyhow::anyhow!("failed to list cron jobs: {e}"))?;
            print!("{}", render_tree(&jobs, Utc::now()));
        }
        CronAction::Enable { id } => {
            store
                .set_enabled(&id, true)
                .await
                .map_err(|e| anyhow::anyhow!("failed to enable {id}: {e}"))?;
            println!("enabled {id}");
        }
        CronAction::Disable { id } => {
            store
                .set_enabled(&id, false)
                .await
                .map_err(|e| anyhow::anyhow!("failed to disable {id}: {e}"))?;
            println!("disabled {id}");
        }
        CronAction::Rm { id } => {
            let removed = store
                .delete(&id)
                .await
                .map_err(|e| anyhow::anyhow!("failed to delete {id}: {e}"))?;
            if removed {
                println!("deleted {id}");
            } else {
                println!("no cron job with id {id}");
            }
        }
    }
    Ok(())
}

/// Render jobs as a `context_from`-nested tree. Roots are jobs with no parent
/// present in the set; each child nests under the first parent it references.
/// Pure: same input + `now` always yields the same string.
fn render_tree(jobs: &[CronJob], now: DateTime<Utc>) -> String {
    if jobs.is_empty() {
        return "No cron jobs scheduled.\n".to_string();
    }

    let ids: std::collections::HashSet<&str> = jobs.iter().map(|j| j.id.as_str()).collect();

    // A job is a root if it depends on nothing we can see (top of a DAG branch).
    let is_root = |j: &CronJob| -> bool {
        j.context_from.is_empty() || !j.context_from.iter().any(|p| ids.contains(p.as_str()))
    };

    let mut out = String::new();
    let mut visited: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for job in jobs.iter().filter(|j| is_root(j)) {
        render_node(job, jobs, now, 0, &mut visited, &mut out);
    }
    // Defensive: surface any job hidden by a dependency cycle so it is never lost.
    let orphans: Vec<&CronJob> = jobs
        .iter()
        .filter(|j| !visited.contains(j.id.as_str()))
        .collect();
    for job in orphans {
        render_node(job, jobs, now, 0, &mut visited, &mut out);
    }
    out
}

fn render_node<'a>(
    job: &'a CronJob,
    jobs: &'a [CronJob],
    now: DateTime<Utc>,
    depth: usize,
    visited: &mut std::collections::HashSet<&'a str>,
    out: &mut String,
) {
    if !visited.insert(job.id.as_str()) {
        return; // already rendered (cycle / shared child)
    }

    let indent = "  ".repeat(depth);
    let glyph = if job.enabled { '●' } else { '○' };
    let name = job.name.as_deref().unwrap_or(&job.id);
    out.push_str(&format!(
        "{indent}{glyph} {name}  {}  ({})  → {}  [{}]\n",
        humanize_until(job.next_run.as_deref(), now),
        schedule_label(&job.schedule),
        job.deliver_to,
        humanize_since(job.last_run.as_deref(), now),
    ));

    // Children are jobs that name this job in their context_from.
    for child in jobs
        .iter()
        .filter(|c| c.context_from.iter().any(|p| p == &job.id))
    {
        render_node(child, jobs, now, depth + 1, visited, out);
    }
}

/// Human label for a schedule cell.
fn schedule_label(schedule: &CronSchedule) -> String {
    match schedule {
        CronSchedule::Cron(expr) => expr.clone(),
        CronSchedule::Human(h) => h.clone(),
        CronSchedule::Once(ts) => format!("once @{ts}"),
    }
}

/// "in 2h" / "in 5m" / "overdue" / "—" — the largest whole unit until `next`.
fn humanize_until(next_run: Option<&str>, now: DateTime<Utc>) -> String {
    let Some(next) = next_run.and_then(parse_rfc3339) else {
        return "—".to_string();
    };
    let secs = (next - now).num_seconds();
    if secs <= 0 {
        return "overdue".to_string();
    }
    format!("in {}", largest_unit(secs))
}

/// "ran 2h ago" / "never" — the largest whole unit since `last`.
fn humanize_since(last_run: Option<&str>, now: DateTime<Utc>) -> String {
    let Some(last) = last_run.and_then(parse_rfc3339) else {
        return "never run".to_string();
    };
    let secs = (now - last).num_seconds();
    if secs <= 0 {
        return "ran just now".to_string();
    }
    format!("ran {} ago", largest_unit(secs))
}

/// Reduce a positive duration in seconds to its largest whole unit: `3d`, `2h`,
/// `5m`, or `<1m`.
fn largest_unit(secs: i64) -> String {
    const MIN: i64 = 60;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    if secs >= DAY {
        format!("{}d", secs / DAY)
    } else if secs >= HOUR {
        format!("{}h", secs / HOUR)
    } else if secs >= MIN {
        format!("{}m", secs / MIN)
    } else {
        "<1m".to_string()
    }
}

fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn job(name: &str, enabled: bool) -> CronJob {
        let mut j = CronJob::new("p", CronSchedule::Cron("0 9 * * *".into()), "s");
        j.name = Some(name.to_string());
        j.enabled = enabled;
        j
    }

    // ── largest_unit / humanize ─────────────────────────────────────────────

    #[test]
    fn largest_unit_picks_biggest_whole() {
        assert_eq!(largest_unit(30), "<1m");
        assert_eq!(largest_unit(90), "1m");
        assert_eq!(largest_unit(3 * 3600), "3h");
        assert_eq!(largest_unit(50 * 3600), "2d");
    }

    #[test]
    fn humanize_until_future_and_overdue() {
        let now = Utc::now();
        let future = (now + chrono::Duration::hours(2)).to_rfc3339();
        assert_eq!(humanize_until(Some(&future), now), "in 2h");

        let past = (now - chrono::Duration::minutes(5)).to_rfc3339();
        assert_eq!(humanize_until(Some(&past), now), "overdue");

        assert_eq!(humanize_until(None, now), "—");
    }

    #[test]
    fn humanize_since_past_and_never() {
        let now = Utc::now();
        let past = (now - chrono::Duration::hours(3)).to_rfc3339();
        assert_eq!(humanize_since(Some(&past), now), "ran 3h ago");
        assert_eq!(humanize_since(None, now), "never run");
    }

    // ── render_tree ─────────────────────────────────────────────────────────

    #[test]
    fn render_empty_is_friendly() {
        assert_eq!(render_tree(&[], Utc::now()), "No cron jobs scheduled.\n");
    }

    #[test]
    fn render_shows_glyphs_for_enabled_and_disabled() {
        let jobs = vec![job("on", true), job("off", false)];
        let out = render_tree(&jobs, Utc::now());
        assert!(out.contains("● on"), "out:\n{out}");
        assert!(out.contains("○ off"), "out:\n{out}");
    }

    #[test]
    fn render_nests_context_from_children() {
        let parent = job("parent", true);
        let mut child = job("child", true);
        child.context_from = vec![parent.id.clone()];
        let parent_id = parent.id.clone();
        let jobs = vec![parent, child];

        let out = render_tree(&jobs, Utc::now());
        let lines: Vec<&str> = out.lines().collect();
        // Parent first at column 0, child indented beneath it.
        assert!(lines[0].starts_with("● parent"), "lines: {lines:?}");
        assert!(
            lines[1].starts_with("  ● child"),
            "child must be indented: {lines:?}"
        );
        // Sanity: the child references the parent we rendered above it.
        assert_eq!(jobs[1].context_from, vec![parent_id]);
    }

    #[test]
    fn render_does_not_lose_jobs_in_a_cycle() {
        // a→b→a cycle: neither is a clean root, but both must still appear once.
        let mut a = job("a", true);
        let mut b = job("b", true);
        a.context_from = vec![b.id.clone()];
        b.context_from = vec![a.id.clone()];
        let jobs = vec![a, b];

        let out = render_tree(&jobs, Utc::now());
        assert_eq!(out.matches("● a").count(), 1, "out:\n{out}");
        assert_eq!(out.matches("● b").count(), 1, "out:\n{out}");
    }

    // ── run_with (DB-backed) ────────────────────────────────────────────────

    async fn store() -> CronStore {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        CronStore::new(db)
    }

    #[tokio::test]
    async fn list_runs_without_error_on_empty_store() {
        let store = store().await;
        run_with(&store, CronAction::List).await.expect("list ok");
    }

    #[tokio::test]
    async fn create_persists_a_job_with_predicted_scope() {
        let store = store().await;
        run_with(
            &store,
            CronAction::Create {
                prompt: "search the web for rust news and post to telegram".into(),
                schedule: "0 9 * * *".into(),
                name: Some("news".into()),
                deliver_to: Some("telegram:me".into()),
                tz: None,
                repeat: None,
                session_id: None,
                context_from: vec![],
            },
        )
        .await
        .expect("create");

        let jobs = store.list().await.expect("list");
        assert_eq!(jobs.len(), 1);
        let j = &jobs[0];
        assert_eq!(j.name.as_deref(), Some("news"));
        assert_eq!(j.deliver_to, "telegram:me");
        assert!(j.granted_scope.tools.contains(&"web_search".to_string()));
        assert!(j.next_run.is_some());
    }

    #[tokio::test]
    async fn enable_disable_and_rm_roundtrip() {
        let store = store().await;
        let created = store
            .create(CronJob::new(
                "p",
                CronSchedule::Cron("0 9 * * *".into()),
                "s",
            ))
            .await
            .expect("create");

        run_with(
            &store,
            CronAction::Disable {
                id: created.id.clone(),
            },
        )
        .await
        .expect("disable");
        let after = store.get(&created.id).await.expect("get").expect("present");
        assert!(!after.enabled);

        run_with(
            &store,
            CronAction::Enable {
                id: created.id.clone(),
            },
        )
        .await
        .expect("enable");
        let after = store.get(&created.id).await.expect("get").expect("present");
        assert!(after.enabled);

        run_with(
            &store,
            CronAction::Rm {
                id: created.id.clone(),
            },
        )
        .await
        .expect("rm");
        assert!(store.get(&created.id).await.expect("get").is_none());
    }
}
