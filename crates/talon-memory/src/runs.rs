//! Per-run execution history for cron jobs (migration v5, web console).
//!
//! One `cron_runs` row per execution attempt. The lifecycle is
//! `insert_running` at dispatch → `finalize` on completion; `record_terminal`
//! covers attempts that never start an agent (skipped / denied). The
//! `cron_jobs.last_run` / `last_output` crash semantics (§4.5) are untouched —
//! this table is additive history, never a second source of scheduling truth.
//!
//! Connection rule (ADR 0004): never hold a `rusqlite::Connection` across an
//! `.await`. Every query runs inside `pool.get().await?.interact(|conn| …)`.

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use rusqlite::{Row, params, types::Type};
use uuid::Uuid;

use crate::cron::fmt_utc;
use crate::{Database, error::MemoryError};

/// Terminal-or-running state of one execution attempt.
/// Stored as lowercase TEXT, constrained by the schema CHECK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Success,
    Failure,
    Timeout,
    Skipped,
    Denied,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Success => "success",
            RunStatus::Failure => "failure",
            RunStatus::Timeout => "timeout",
            RunStatus::Skipped => "skipped",
            RunStatus::Denied => "denied",
        }
    }
}

impl FromStr for RunStatus {
    type Err = MemoryError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "running" => Ok(RunStatus::Running),
            "success" => Ok(RunStatus::Success),
            "failure" => Ok(RunStatus::Failure),
            "timeout" => Ok(RunStatus::Timeout),
            "skipped" => Ok(RunStatus::Skipped),
            "denied" => Ok(RunStatus::Denied),
            other => Err(MemoryError::Cron(format!("unknown run status: {other}"))),
        }
    }
}

/// One execution attempt of a cron job. Mirrors the `cron_runs` table.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CronRun {
    pub id: String,
    pub job_id: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: RunStatus,
    pub output: Option<String>,
    pub error: Option<String>,
    /// Compact JSON transcript of the run's `AgentEvent`s, for the run-detail view.
    pub events_json: Option<String>,
}

impl CronRun {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let raw: String = row.get("status")?;
        let status = RunStatus::from_str(&raw).map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unknown run status: {raw}"),
                )),
            )
        })?;
        Ok(Self {
            id: row.get("id")?,
            job_id: row.get("job_id")?,
            started_at: row.get("started_at")?,
            finished_at: row.get("finished_at")?,
            status,
            output: row.get("output")?,
            error: row.get("error")?,
            events_json: row.get("events_json")?,
        })
    }
}

/// Persisted run-history store backed by the shared SQLite `Database`.
#[derive(Clone)]
pub struct RunStore {
    db: Arc<Database>,
}

impl RunStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Open an attempt: insert a `running` row at dispatch time and return it.
    pub async fn insert_running(
        &self,
        job_id: &str,
        started_at: DateTime<Utc>,
    ) -> Result<CronRun, MemoryError> {
        let run = CronRun {
            id: Uuid::new_v4().to_string(),
            job_id: job_id.to_string(),
            started_at: fmt_utc(started_at),
            finished_at: None,
            status: RunStatus::Running,
            output: None,
            error: None,
            events_json: None,
        };
        let r = run.clone();
        self.db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<()> {
                conn.execute(
                    "INSERT INTO cron_runs (id, job_id, started_at, status)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![r.id, r.job_id, r.started_at, r.status.as_str()],
                )?;
                Ok(())
            })
            .await??;
        Ok(run)
    }

    /// Close an attempt: set its terminal status, `finished_at = now`, and any
    /// output / error / transcript. Only a row still in `running` is updated —
    /// a second finalize is a no-op (`Ok(false)`), so crash-racing writers can
    /// never flip a terminal status.
    pub async fn finalize(
        &self,
        id: &str,
        status: RunStatus,
        output: Option<String>,
        error: Option<String>,
        events_json: Option<String>,
    ) -> Result<bool, MemoryError> {
        if status == RunStatus::Running {
            return Err(MemoryError::Cron(
                "finalize requires a terminal status, got 'running'".to_string(),
            ));
        }
        let id = id.to_string();
        let finished = fmt_utc(Utc::now());
        let updated = self
            .db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<usize> {
                conn.execute(
                    "UPDATE cron_runs
                       SET status = ?2, finished_at = ?3, output = ?4,
                           error = ?5, events_json = ?6
                     WHERE id = ?1 AND status = 'running'",
                    params![id, status.as_str(), finished, output, error, events_json],
                )
            })
            .await??;
        Ok(updated > 0)
    }

    /// Record an attempt that never started an agent (e.g. `skipped` by the
    /// missed-run policy, `denied` by the approval membrane before launch).
    /// The row is inserted already-terminal with `finished_at = started_at`.
    pub async fn record_terminal(
        &self,
        job_id: &str,
        status: RunStatus,
        error: Option<String>,
    ) -> Result<CronRun, MemoryError> {
        if status == RunStatus::Running {
            return Err(MemoryError::Cron(
                "record_terminal requires a terminal status, got 'running'".to_string(),
            ));
        }
        let now = fmt_utc(Utc::now());
        let run = CronRun {
            id: Uuid::new_v4().to_string(),
            job_id: job_id.to_string(),
            started_at: now.clone(),
            finished_at: Some(now),
            status,
            output: None,
            error,
            events_json: None,
        };
        let r = run.clone();
        self.db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<()> {
                conn.execute(
                    "INSERT INTO cron_runs
                       (id, job_id, started_at, finished_at, status, error)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![r.id, r.job_id, r.started_at, r.finished_at, r.status.as_str(), r.error],
                )?;
                Ok(())
            })
            .await??;
        Ok(run)
    }

    /// Fetch a single run by id.
    pub async fn get(&self, id: &str) -> Result<Option<CronRun>, MemoryError> {
        let id = id.to_string();
        let run = self
            .db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<Option<CronRun>> {
                let mut stmt = conn.prepare("SELECT * FROM cron_runs WHERE id = ?1")?;
                let mut rows = stmt.query_map([id], CronRun::from_row)?;
                match rows.next() {
                    Some(row) => Ok(Some(row?)),
                    None => Ok(None),
                }
            })
            .await??;
        Ok(run)
    }

    /// Run history for one job, newest first. `started_at` has whole-second
    /// precision, so `rowid` (insertion order) breaks same-second ties.
    pub async fn list_for_job(
        &self,
        job_id: &str,
        limit: i64,
    ) -> Result<Vec<CronRun>, MemoryError> {
        let job_id = job_id.to_string();
        let runs = self
            .db
            .pool()
            .get()
            .await?
            .interact(move |conn| -> rusqlite::Result<Vec<CronRun>> {
                let mut stmt = conn.prepare(
                    "SELECT * FROM cron_runs WHERE job_id = ?1
                     ORDER BY started_at DESC, rowid DESC LIMIT ?2",
                )?;
                stmt.query_map(params![job_id, limit], CronRun::from_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await??;
        Ok(runs)
    }

    /// The most recent run of every job that has ever run — one row per job.
    /// Drives the graph node colors in a single query.
    pub async fn latest_per_job(&self) -> Result<Vec<CronRun>, MemoryError> {
        let runs = self
            .db
            .pool()
            .get()
            .await?
            .interact(|conn| -> rusqlite::Result<Vec<CronRun>> {
                let mut stmt = conn.prepare(
                    "SELECT * FROM cron_runs r
                     WHERE rowid = (SELECT MAX(rowid) FROM cron_runs WHERE job_id = r.job_id)",
                )?;
                stmt.query_map([], CronRun::from_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await??;
        Ok(runs)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{CronJob, CronSchedule, CronStore};

    async fn stores() -> (CronStore, RunStore) {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        (CronStore::new(Arc::clone(&db)), RunStore::new(db))
    }

    async fn make_job(cron: &CronStore, name: &str) -> CronJob {
        cron.create(
            CronJob::new("prompt", CronSchedule::Human("daily".into()), "s").with_name(name),
        )
        .await
        .expect("create job")
    }

    #[tokio::test]
    async fn insert_running_then_finalize_success() {
        let (cron, runs) = stores().await;
        let job = make_job(&cron, "a").await;

        let run = runs
            .insert_running(&job.id, Utc::now())
            .await
            .expect("insert");
        assert_eq!(run.status, RunStatus::Running);
        assert!(run.finished_at.is_none());

        let updated = runs
            .finalize(
                &run.id,
                RunStatus::Success,
                Some("out".into()),
                None,
                Some("[]".into()),
            )
            .await
            .expect("finalize");
        assert!(updated);

        let after = runs.get(&run.id).await.expect("get").expect("present");
        assert_eq!(after.status, RunStatus::Success);
        assert_eq!(after.output.as_deref(), Some("out"));
        assert_eq!(after.events_json.as_deref(), Some("[]"));
        assert!(after.finished_at.is_some());
    }

    #[tokio::test]
    async fn finalize_failure_records_error() {
        let (cron, runs) = stores().await;
        let job = make_job(&cron, "a").await;
        let run = runs
            .insert_running(&job.id, Utc::now())
            .await
            .expect("insert");

        runs.finalize(
            &run.id,
            RunStatus::Failure,
            None,
            Some("llm exploded".into()),
            None,
        )
        .await
        .expect("finalize");

        let after = runs.get(&run.id).await.expect("get").expect("present");
        assert_eq!(after.status, RunStatus::Failure);
        assert_eq!(after.error.as_deref(), Some("llm exploded"));
        assert!(after.output.is_none());
    }

    #[tokio::test]
    async fn finalize_twice_is_noop() {
        let (cron, runs) = stores().await;
        let job = make_job(&cron, "a").await;
        let run = runs
            .insert_running(&job.id, Utc::now())
            .await
            .expect("insert");

        let first = runs
            .finalize(&run.id, RunStatus::Success, Some("v1".into()), None, None)
            .await
            .expect("first");
        assert!(first);

        let second = runs
            .finalize(
                &run.id,
                RunStatus::Failure,
                None,
                Some("late writer".into()),
                None,
            )
            .await
            .expect("second");
        assert!(!second, "second finalize must be a no-op");

        let after = runs.get(&run.id).await.expect("get").expect("present");
        assert_eq!(after.status, RunStatus::Success, "terminal status sticks");
        assert_eq!(after.output.as_deref(), Some("v1"));
    }

    #[tokio::test]
    async fn finalize_rejects_running_status() {
        let (cron, runs) = stores().await;
        let job = make_job(&cron, "a").await;
        let run = runs
            .insert_running(&job.id, Utc::now())
            .await
            .expect("insert");
        assert!(
            runs.finalize(&run.id, RunStatus::Running, None, None, None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn record_terminal_skipped() {
        let (cron, runs) = stores().await;
        let job = make_job(&cron, "a").await;
        let run = runs
            .record_terminal(&job.id, RunStatus::Skipped, Some("missed-run policy".into()))
            .await
            .expect("record");
        let after = runs.get(&run.id).await.expect("get").expect("present");
        assert_eq!(after.status, RunStatus::Skipped);
        assert_eq!(after.finished_at, Some(after.started_at.clone()));
    }

    #[tokio::test]
    async fn list_for_job_newest_first_with_limit() {
        let (cron, runs) = stores().await;
        let job = make_job(&cron, "a").await;
        let other = make_job(&cron, "b").await;

        let mut ids = Vec::new();
        for _ in 0..3 {
            let r = runs
                .insert_running(&job.id, Utc::now())
                .await
                .expect("insert");
            ids.push(r.id);
        }
        runs.insert_running(&other.id, Utc::now())
            .await
            .expect("other job run");

        let all = runs.list_for_job(&job.id, 10).await.expect("list");
        assert_eq!(all.len(), 3, "only this job's runs");
        assert_eq!(all[0].id, ids[2], "newest first (rowid tiebreak)");
        assert_eq!(all[2].id, ids[0]);

        let limited = runs.list_for_job(&job.id, 2).await.expect("list limited");
        assert_eq!(limited.len(), 2);
    }

    #[tokio::test]
    async fn latest_per_job_one_row_per_job() {
        let (cron, runs) = stores().await;
        let a = make_job(&cron, "a").await;
        let b = make_job(&cron, "b").await;

        runs.insert_running(&a.id, Utc::now()).await.expect("a1");
        let a2 = runs.insert_running(&a.id, Utc::now()).await.expect("a2");
        let b1 = runs.insert_running(&b.id, Utc::now()).await.expect("b1");
        runs.finalize(&b1.id, RunStatus::Failure, None, Some("boom".into()), None)
            .await
            .expect("finalize b1");

        let latest = runs.latest_per_job().await.expect("latest");
        assert_eq!(latest.len(), 2);
        let for_a = latest.iter().find(|r| r.job_id == a.id).expect("a entry");
        assert_eq!(for_a.id, a2.id, "latest run wins");
        let for_b = latest.iter().find(|r| r.job_id == b.id).expect("b entry");
        assert_eq!(for_b.status, RunStatus::Failure);
    }

    #[tokio::test]
    async fn empty_table_queries_are_clean() {
        let (cron, runs) = stores().await;
        let job = make_job(&cron, "a").await;

        assert!(runs.get("nope").await.expect("get").is_none());
        assert!(
            runs.list_for_job(&job.id, 10)
                .await
                .expect("list")
                .is_empty()
        );
        assert!(runs.latest_per_job().await.expect("latest").is_empty());
        assert!(
            !runs
                .finalize("nope", RunStatus::Success, None, None, None)
                .await
                .expect("finalize missing")
        );
    }

    #[tokio::test]
    async fn deleting_job_cascades_runs() {
        let (cron, runs) = stores().await;
        let job = make_job(&cron, "a").await;
        let run = runs
            .insert_running(&job.id, Utc::now())
            .await
            .expect("insert");

        assert!(cron.delete(&job.id).await.expect("delete job"));
        assert!(
            runs.get(&run.id).await.expect("get").is_none(),
            "ON DELETE CASCADE removes runs"
        );
    }

    #[tokio::test]
    async fn insert_running_rejects_unknown_job() {
        let (_cron, runs) = stores().await;
        assert!(
            runs.insert_running("no-such-job", Utc::now()).await.is_err(),
            "FK constraint rejects orphan runs"
        );
    }
}
