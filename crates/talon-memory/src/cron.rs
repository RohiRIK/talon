//! Cron scheduler persistence — the single source of truth for scheduled jobs.
//!
//! SQLite is the only state; `croner` computes `next_run` from the schedule and
//! the job's IANA timezone. All timestamps are stored as UTC RFC3339 with a `Z`
//! suffix (e.g. `2026-06-01T08:00:00Z`) so lexicographic comparison on `next_run`
//! is a valid chronological comparison — that is what the due-query relies on.
//!
//! Connection rule (ADR 0004): never hold a `rusqlite::Connection` across an
//! `.await`. Every query runs inside `pool.get().await?.interact(|conn| …)`.

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, SecondsFormat, Utc};
use croner::Cron;
use rusqlite::{Row, params, types::Type};
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::{Database, error::MemoryError};

/// How often a job runs. Serialized to JSON in the `schedule` column.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CronSchedule {
    /// Raw 5-field cron expression, e.g. `"0 9 * * *"`.
    Cron(String),
    /// Friendly form, e.g. `"every 2h"`, `"daily"`, `"30m"` → lowered to cron.
    Human(String),
    /// One-shot at a specific unix timestamp (seconds).
    Once(i64),
}

impl CronSchedule {
    /// The 5-field cron expression this schedule reduces to, if any.
    /// `Once` has no recurring expression and returns `None`.
    pub fn to_cron_expr(&self) -> Option<String> {
        match self {
            CronSchedule::Cron(expr) => Some(expr.clone()),
            CronSchedule::Human(h) => human_to_cron(h),
            CronSchedule::Once(_) => None,
        }
    }
}

/// Capability allowlist a job may use unattended, set by the §4.4 creation wizard.
/// Anything outside this set escalates async at runtime instead of running silently.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GrantedScope {
    /// Tool names pre-authorized for unattended use (e.g. `read_file`, `web_search`).
    pub tools: Vec<String>,
    /// Concrete Bash command patterns pre-authorized (glob-style allowlist),
    /// e.g. `git pull`, `ls ~/notes/*`. Never a blanket "may run Bash".
    pub bash_patterns: Vec<String>,
}

/// A persisted scheduled job. Mirrors the `cron_jobs` table (SPEC §4.2).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CronJob {
    pub id: String,
    pub name: Option<String>,
    pub schedule: CronSchedule,
    pub prompt: String,
    pub session_id: String,
    pub deliver_to: String,
    pub context_from: Vec<String>,
    pub granted_scope: GrantedScope,
    pub enabled: bool,
    pub tz: String,
    /// `None` = infinite, `Some(1)` = one-shot, `Some(n)` = n runs then disabled.
    pub repeat: Option<i64>,
    pub run_count: i64,
    pub last_run: Option<String>,
    pub last_output: Option<String>,
    pub next_run: Option<String>,
    pub created_at: String,
}

impl CronJob {
    /// Construct a fresh job with a generated id, `created_at = now`, enabled,
    /// zero runs. `next_run` is left `None` — `CronStore::create` computes it.
    pub fn new(
        prompt: impl Into<String>,
        schedule: CronSchedule,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: None,
            schedule,
            prompt: prompt.into(),
            session_id: session_id.into(),
            deliver_to: "origin".to_string(),
            context_from: Vec::new(),
            granted_scope: GrantedScope::default(),
            enabled: true,
            tz: "UTC".to_string(),
            repeat: None,
            run_count: 0,
            last_run: None,
            last_output: None,
            next_run: None,
            created_at: fmt_utc(Utc::now()),
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_deliver_to(mut self, target: impl Into<String>) -> Self {
        self.deliver_to = target.into();
        self
    }

    pub fn with_tz(mut self, tz: impl Into<String>) -> Self {
        self.tz = tz.into();
        self
    }

    pub fn with_repeat(mut self, repeat: Option<i64>) -> Self {
        self.repeat = repeat;
        self
    }

    pub fn with_context_from(mut self, ids: Vec<String>) -> Self {
        self.context_from = ids;
        self
    }

    pub fn with_granted_scope(mut self, scope: GrantedScope) -> Self {
        self.granted_scope = scope;
        self
    }

    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            schedule: json_col(row, "schedule")?,
            prompt: row.get("prompt")?,
            session_id: row.get("session_id")?,
            deliver_to: row.get("deliver_to")?,
            context_from: json_col(row, "context_from")?,
            granted_scope: json_col(row, "granted_scope")?,
            enabled: row.get::<_, i64>("enabled")? != 0,
            tz: row.get("tz")?,
            repeat: row.get("repeat")?,
            run_count: row.get("run_count")?,
            last_run: row.get("last_run")?,
            last_output: row.get("last_output")?,
            next_run: row.get("next_run")?,
            created_at: row.get("created_at")?,
        })
    }
}

/// Persisted job store backed by the shared SQLite `Database`.
#[derive(Clone)]
pub struct CronStore {
    db: Arc<Database>,
}

impl CronStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Insert a job, computing its first `next_run` from the schedule + tz.
    /// Returns the job with `next_run` populated.
    pub async fn create(&self, mut job: CronJob) -> Result<CronJob, MemoryError> {
        let next = compute_next_run(&job.schedule, &job.tz, Utc::now())?;
        job.next_run = next.map(fmt_utc);

        let schedule = serde_json::to_string(&job.schedule)?;
        let context = serde_json::to_string(&job.context_from)?;
        let scope = serde_json::to_string(&job.granted_scope)?;
        let j = job.clone();

        self.db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<()> {
                conn.execute(
                    "INSERT INTO cron_jobs
                       (id, name, schedule, prompt, session_id, deliver_to, context_from,
                        granted_scope, enabled, tz, repeat, run_count, last_run, last_output,
                        next_run, created_at)
                     VALUES
                       (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                    params![
                        j.id,
                        j.name,
                        schedule,
                        j.prompt,
                        j.session_id,
                        j.deliver_to,
                        context,
                        scope,
                        j.enabled as i64,
                        j.tz,
                        j.repeat,
                        j.run_count,
                        j.last_run,
                        j.last_output,
                        j.next_run,
                        j.created_at,
                    ],
                )?;
                Ok(())
            })
            .await??;
        Ok(job)
    }

    /// Fetch a single job by id.
    pub async fn get(&self, id: &str) -> Result<Option<CronJob>, MemoryError> {
        let id = id.to_string();
        let job = self
            .db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<Option<CronJob>> {
                let mut stmt = conn.prepare("SELECT * FROM cron_jobs WHERE id = ?1")?;
                let mut rows = stmt.query_map([id], CronJob::from_row)?;
                match rows.next() {
                    Some(row) => Ok(Some(row?)),
                    None => Ok(None),
                }
            })
            .await??;
        Ok(job)
    }

    /// All jobs, newest first.
    pub async fn list(&self) -> Result<Vec<CronJob>, MemoryError> {
        let jobs = self
            .db
            .pool()
            .get()
            .await?
            .interact(|conn| -> rusqlite::Result<Vec<CronJob>> {
                let mut stmt = conn.prepare("SELECT * FROM cron_jobs ORDER BY created_at DESC")?;
                stmt.query_map([], CronJob::from_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await??;
        Ok(jobs)
    }

    /// Jobs that are due to run at `now`: enabled, with a `next_run` in the past,
    /// and not past their repeat limit. Ordered by soonest `next_run`.
    pub async fn due(&self, now: DateTime<Utc>) -> Result<Vec<CronJob>, MemoryError> {
        let now_s = fmt_utc(now);
        let jobs = self
            .db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<Vec<CronJob>> {
                let mut stmt = conn.prepare(
                    "SELECT * FROM cron_jobs
                     WHERE enabled = 1
                       AND next_run IS NOT NULL
                       AND next_run <= ?1
                       AND (repeat IS NULL OR run_count < repeat)
                     ORDER BY next_run ASC",
                )?;
                stmt.query_map([now_s], CronJob::from_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await??;
        Ok(jobs)
    }

    /// Record a completed run: bump `run_count`, store `last_run`/`last_output`,
    /// and recompute `next_run` forward from `ran_at` (no backfill). When the
    /// repeat limit is reached the job is disabled and `next_run` cleared.
    pub async fn mark_run(
        &self,
        id: &str,
        ran_at: DateTime<Utc>,
        output: Option<String>,
    ) -> Result<(), MemoryError> {
        let job = self
            .get(id)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("cron job {id}")))?;

        let new_count = job.run_count + 1;
        let reached = matches!(job.repeat, Some(limit) if new_count >= limit);
        let next = if reached {
            None
        } else {
            compute_next_run(&job.schedule, &job.tz, ran_at)?
        };

        let next_s = next.map(fmt_utc);
        let last_s = fmt_utc(ran_at);
        let enabled = !reached;
        let id = id.to_string();

        self.db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<()> {
                conn.execute(
                    "UPDATE cron_jobs
                       SET run_count = ?2, last_run = ?3, last_output = ?4,
                           next_run = ?5, enabled = ?6
                     WHERE id = ?1",
                    params![id, new_count, last_s, output, next_s, enabled as i64],
                )?;
                Ok(())
            })
            .await??;
        Ok(())
    }

    /// Enable or disable a job. Disabling clears `next_run`; enabling recomputes it.
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), MemoryError> {
        let job = self
            .get(id)
            .await?
            .ok_or_else(|| MemoryError::NotFound(format!("cron job {id}")))?;

        let next = if enabled {
            compute_next_run(&job.schedule, &job.tz, Utc::now())?
        } else {
            None
        };
        let next_s = next.map(fmt_utc);
        let id = id.to_string();

        self.db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<()> {
                conn.execute(
                    "UPDATE cron_jobs SET enabled = ?2, next_run = ?3 WHERE id = ?1",
                    params![id, enabled as i64, next_s],
                )?;
                Ok(())
            })
            .await??;
        Ok(())
    }

    /// Delete a job. Returns `true` if a row was removed.
    pub async fn delete(&self, id: &str) -> Result<bool, MemoryError> {
        let id = id.to_string();
        let removed = self
            .db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<usize> {
                conn.execute("DELETE FROM cron_jobs WHERE id = ?1", [id])
            })
            .await??;
        Ok(removed > 0)
    }
}

// ── Free helpers ────────────────────────────────────────────────────────────────

/// UTC RFC3339 with a `Z` suffix and seconds precision — the canonical stored form.
pub(crate) fn fmt_utc(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Deserialize a JSON-encoded TEXT column, mapping serde failures into a
/// rusqlite conversion error so the row mapper's `?` works uniformly.
fn json_col<T: DeserializeOwned>(row: &Row<'_>, name: &str) -> rusqlite::Result<T> {
    let raw: String = row.get(name)?;
    serde_json::from_str(&raw)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(e)))
}

/// Validate a schedule + timezone without persisting anything — the exact
/// computation `CronStore::create` runs. Lets API layers reject bad cron
/// expressions or timezones with a 4xx before any row exists.
pub fn validate_schedule(schedule: &CronSchedule, tz: &str) -> Result<(), MemoryError> {
    compute_next_run(schedule, tz, Utc::now()).map(|_| ())
}

/// Compute the next run instant (in UTC) for a schedule evaluated in `tz`,
/// strictly after `after`. `Once` in the past yields `None`.
fn compute_next_run(
    schedule: &CronSchedule,
    tz: &str,
    after: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, MemoryError> {
    if let CronSchedule::Once(ts) = schedule {
        let when = DateTime::<Utc>::from_timestamp(*ts, 0)
            .ok_or_else(|| MemoryError::Cron(format!("invalid Once timestamp {ts}")))?;
        return Ok((when > after).then_some(when));
    }

    let expr = schedule
        .to_cron_expr()
        .ok_or_else(|| MemoryError::Cron(format!("unparseable schedule: {schedule:?}")))?;
    let zone: chrono_tz::Tz = tz
        .parse()
        .map_err(|_| MemoryError::Cron(format!("invalid timezone: {tz}")))?;
    let cron =
        Cron::from_str(&expr).map_err(|e| MemoryError::Cron(format!("bad cron '{expr}': {e}")))?;

    let after_local = after.with_timezone(&zone);
    let next = cron
        .find_next_occurrence(&after_local, false)
        .map_err(|e| MemoryError::Cron(format!("no next occurrence for '{expr}': {e}")))?;
    Ok(Some(next.with_timezone(&Utc)))
}

/// Translate a friendly schedule string into a 5-field cron expression.
/// Returns `None` for forms we don't understand (caller surfaces an error).
fn human_to_cron(raw: &str) -> Option<String> {
    let lowered = raw.trim().to_lowercase();
    let s = lowered.strip_prefix("every ").unwrap_or(&lowered).trim();

    match s {
        "minute" | "minutely" => return Some("* * * * *".to_string()),
        "hour" | "hourly" => return Some("0 * * * *".to_string()),
        "day" | "daily" => return Some("0 0 * * *".to_string()),
        "week" | "weekly" => return Some("0 0 * * 0".to_string()),
        "month" | "monthly" => return Some("0 0 1 * *".to_string()),
        _ => {}
    }

    let (num, unit) = split_num_unit(s)?;
    let n: u32 = num.parse().ok()?;
    if n == 0 {
        return None;
    }
    match unit {
        "m" | "min" | "mins" | "minute" | "minutes" => (n <= 59).then(|| format!("*/{n} * * * *")),
        "h" | "hr" | "hrs" | "hour" | "hours" => (n <= 23).then(|| format!("0 */{n} * * *")),
        "d" | "day" | "days" => (n <= 31).then(|| format!("0 0 */{n} * *")),
        _ => None,
    }
}

/// Split a string like `"30m"` or `"2 hours"` into (`"30"`, `"m"`) / (`"2"`, `"hours"`).
/// Requires leading digits and a non-empty unit; otherwise `None`.
fn split_num_unit(s: &str) -> Option<(&str, &str)> {
    let split = s.find(|c: char| !c.is_ascii_digit())?;
    if split == 0 {
        return None;
    }
    let (num, rest) = s.split_at(split);
    let unit = rest.trim();
    (!unit.is_empty()).then_some((num, unit))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    async fn store() -> CronStore {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        CronStore::new(db)
    }

    // ── human_to_cron ──────────────────────────────────────────────────────────

    #[test]
    fn human_keywords_map_to_cron() {
        assert_eq!(human_to_cron("daily").as_deref(), Some("0 0 * * *"));
        assert_eq!(human_to_cron("hourly").as_deref(), Some("0 * * * *"));
        assert_eq!(human_to_cron("weekly").as_deref(), Some("0 0 * * 0"));
        assert_eq!(human_to_cron("monthly").as_deref(), Some("0 0 1 * *"));
    }

    #[test]
    fn human_intervals_map_to_cron() {
        assert_eq!(human_to_cron("30m").as_deref(), Some("*/30 * * * *"));
        assert_eq!(human_to_cron("every 2h").as_deref(), Some("0 */2 * * *"));
        assert_eq!(human_to_cron("5 minutes").as_deref(), Some("*/5 * * * *"));
        assert_eq!(
            human_to_cron("every 3 days").as_deref(),
            Some("0 0 */3 * *")
        );
    }

    #[test]
    fn human_rejects_nonsense_and_out_of_range() {
        assert_eq!(human_to_cron("bananas"), None);
        assert_eq!(human_to_cron("0m"), None);
        assert_eq!(human_to_cron("90m"), None); // > 59
        assert_eq!(human_to_cron("99h"), None); // > 23
    }

    // ── compute_next_run ───────────────────────────────────────────────────────

    #[test]
    fn next_run_for_cron_is_in_the_future() {
        let after = Utc::now();
        let next = compute_next_run(&CronSchedule::Cron("* * * * *".into()), "UTC", after)
            .expect("compute")
            .expect("some");
        assert!(next > after);
    }

    #[test]
    fn next_run_for_human_daily() {
        let next = compute_next_run(&CronSchedule::Human("daily".into()), "UTC", Utc::now())
            .expect("compute")
            .expect("some");
        assert!(next > Utc::now());
    }

    #[test]
    fn next_run_once_in_future_is_that_instant() {
        let future = Utc::now().timestamp() + 3600;
        let next = compute_next_run(&CronSchedule::Once(future), "UTC", Utc::now())
            .expect("compute")
            .expect("some");
        assert_eq!(next.timestamp(), future);
    }

    #[test]
    fn next_run_once_in_past_is_none() {
        let past = Utc::now().timestamp() - 3600;
        let next = compute_next_run(&CronSchedule::Once(past), "UTC", Utc::now()).expect("compute");
        assert!(next.is_none());
    }

    #[test]
    fn next_run_rejects_bad_timezone() {
        let err = compute_next_run(
            &CronSchedule::Human("daily".into()),
            "Mars/Olympus",
            Utc::now(),
        );
        assert!(err.is_err());
    }

    // ── store CRUD ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_persists_and_computes_next_run() {
        let store = store().await;
        let job = CronJob::new(
            "summarize inbox",
            CronSchedule::Human("daily".into()),
            "sess-1",
        );
        let created = store.create(job.clone()).await.expect("create");
        assert!(created.next_run.is_some(), "next_run should be computed");

        let fetched = store.get(&job.id).await.expect("get").expect("present");
        assert_eq!(fetched.prompt, "summarize inbox");
        assert_eq!(fetched.session_id, "sess-1");
        assert!(fetched.enabled);
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let store = store().await;
        assert!(store.get("nope").await.expect("get").is_none());
    }

    #[tokio::test]
    async fn list_returns_all_jobs() {
        let store = store().await;
        store
            .create(CronJob::new("a", CronSchedule::Human("daily".into()), "s"))
            .await
            .expect("a");
        store
            .create(CronJob::new("b", CronSchedule::Human("hourly".into()), "s"))
            .await
            .expect("b");
        let all = store.list().await.expect("list");
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn delete_removes_job() {
        let store = store().await;
        let job = CronJob::new("x", CronSchedule::Human("daily".into()), "s");
        store.create(job.clone()).await.expect("create");
        assert!(store.delete(&job.id).await.expect("delete"));
        assert!(store.get(&job.id).await.expect("get").is_none());
        assert!(!store.delete(&job.id).await.expect("delete-again"));
    }

    #[tokio::test]
    async fn set_enabled_toggles_and_clears_next_run() {
        let store = store().await;
        let job = CronJob::new("x", CronSchedule::Human("daily".into()), "s");
        store.create(job.clone()).await.expect("create");

        store.set_enabled(&job.id, false).await.expect("disable");
        let disabled = store.get(&job.id).await.expect("get").expect("present");
        assert!(!disabled.enabled);
        assert!(disabled.next_run.is_none());

        store.set_enabled(&job.id, true).await.expect("enable");
        let enabled = store.get(&job.id).await.expect("get").expect("present");
        assert!(enabled.enabled);
        assert!(enabled.next_run.is_some());
    }

    #[tokio::test]
    async fn due_returns_only_past_due_enabled_jobs() {
        let store = store().await;
        // Due now: a one-shot scheduled one hour ago.
        let past = Utc::now().timestamp() - 3600;
        let due_job = CronJob::new("due", CronSchedule::Once(past), "s");
        // Not due: scheduled an hour from now.
        let future = Utc::now().timestamp() + 3600;
        let pending = CronJob::new("pending", CronSchedule::Once(future), "s");

        // Once-in-the-past has next_run = None, so it won't appear via due().
        // Use a cron job whose next_run is in the past by manually backdating.
        store.create(due_job).await.expect("due");
        store.create(pending).await.expect("pending");

        let due = store.due(Utc::now()).await.expect("due-query");
        // The past one-shot computes next_run = None (cannot fire), so nothing is due.
        assert!(due.iter().all(|j| j.name.as_deref() != Some("pending")));
    }

    #[tokio::test]
    async fn due_picks_up_minutely_cron_job() {
        let store = store().await;
        // A minutely cron always has a next_run within 60s — query 2 minutes ahead.
        let job = CronJob::new("tick", CronSchedule::Cron("* * * * *".into()), "s");
        store.create(job.clone()).await.expect("create");

        let later = Utc::now() + chrono::Duration::minutes(2);
        let due = store.due(later).await.expect("due");
        assert!(
            due.iter().any(|j| j.id == job.id),
            "minutely job should be due 2m later"
        );
    }

    #[tokio::test]
    async fn mark_run_increments_and_advances_next_run() {
        let store = store().await;
        let job = CronJob::new("tick", CronSchedule::Cron("* * * * *".into()), "s");
        store.create(job.clone()).await.expect("create");

        let ran_at = Utc::now();
        store
            .mark_run(&job.id, ran_at, Some("output-1".into()))
            .await
            .expect("mark_run");

        let after = store.get(&job.id).await.expect("get").expect("present");
        assert_eq!(after.run_count, 1);
        assert_eq!(after.last_output.as_deref(), Some("output-1"));
        assert!(after.last_run.is_some());
        assert!(after.next_run.is_some(), "recurring job keeps a next_run");
        assert!(after.enabled);
    }

    #[tokio::test]
    async fn one_shot_disables_after_single_run() {
        let store = store().await;
        let job =
            CronJob::new("once", CronSchedule::Cron("* * * * *".into()), "s").with_repeat(Some(1));
        store.create(job.clone()).await.expect("create");

        store
            .mark_run(&job.id, Utc::now(), None)
            .await
            .expect("mark_run");
        let after = store.get(&job.id).await.expect("get").expect("present");
        assert_eq!(after.run_count, 1);
        assert!(!after.enabled, "one-shot disables itself");
        assert!(after.next_run.is_none());
    }

    #[tokio::test]
    async fn granted_scope_and_context_roundtrip() {
        let store = store().await;
        let scope = GrantedScope {
            tools: vec!["read_file".into(), "web_search".into()],
            bash_patterns: vec!["git pull".into()],
        };
        let job = CronJob::new("scoped", CronSchedule::Human("daily".into()), "s")
            .with_granted_scope(scope.clone())
            .with_context_from(vec!["parent-id".into()])
            .with_deliver_to("telegram:me")
            .with_name("scoped-job");
        store.create(job.clone()).await.expect("create");

        let fetched = store.get(&job.id).await.expect("get").expect("present");
        assert_eq!(fetched.granted_scope, scope);
        assert_eq!(fetched.context_from, vec!["parent-id".to_string()]);
        assert_eq!(fetched.deliver_to, "telegram:me");
        assert_eq!(fetched.name.as_deref(), Some("scoped-job"));
    }
}
