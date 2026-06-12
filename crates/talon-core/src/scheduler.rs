//! Self-rolled cron tick-loop (SPEC §4.3). SQLite (`CronStore`) is the only
//! source of truth; `croner` computes `next_run`. There is no external scheduler
//! state and no `tokio-cron-scheduler` — just a tokio interval that, on each
//! tick, asks the store which jobs are due and dispatches them.
//!
//! The actual work of a job — building an `Agent`, running it, and routing its
//! output to `deliver_to` — lives behind the [`JobRunner`] seam so this crate
//! stays free of any gateway dependency (the binary wires the real runner).
//!
//! Design rules honored here:
//! - A slow job never blocks the ticker: each due job runs in its own task,
//!   tracked in a [`TaskTracker`] so the daemon can drain in-flight jobs on
//!   shutdown.
//! - No double-dispatch: a job already running is skipped until it completes
//!   (an in-memory guard, deliberately *not* persisted — a process crash drops
//!   the guard so the job re-fires on restart, per the §4.5 crash policy).
//! - No backfill: `next_run` is recomputed forward from the moment the job ran.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use talon_memory::{CronJob, CronStore, RunStatus, RunStore};

/// Live event feed for the web console (SSE). Emitted by the scheduler around
/// each run, by the daemon's runner for approvals, and by web handlers on job
/// mutations. `broadcast` semantics: slow subscribers lose old events, never
/// block the scheduler.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    RunStarted {
        job_id: String,
        run_id: String,
    },
    RunFinished {
        job_id: String,
        run_id: String,
        status: RunStatus,
    },
    JobChanged {
        job_id: String,
    },
    ApprovalPending {
        call_id: String,
        job_id: Option<String>,
        tool: String,
        args: serde_json::Value,
    },
    ApprovalResolved {
        call_id: String,
        approved: bool,
    },
}

/// Result of executing one scheduled job.
#[derive(Debug, Clone, Default)]
pub struct JobOutcome {
    /// The job's final textual output, stored as `last_output` for downstream
    /// `context_from` injection. `None` on failure.
    pub output: Option<String>,
    /// Whether the run completed successfully. Currently informational — the
    /// store advances `next_run` on any *completion* (only a process crash,
    /// which skips `mark_run` entirely, causes a re-fire).
    pub success: bool,
    /// Why the run failed, recorded in `cron_runs.error`. `None` on success.
    pub error: Option<String>,
    /// Compact JSON transcript of the run's `AgentEvent`s, recorded in
    /// `cron_runs.events_json` for the web console run-detail view.
    pub events_json: Option<String>,
}

impl JobOutcome {
    pub fn ok(output: Option<String>) -> Self {
        Self {
            output,
            success: true,
            error: None,
            events_json: None,
        }
    }

    pub fn failed() -> Self {
        Self {
            output: None,
            success: false,
            error: None,
            events_json: None,
        }
    }

    pub fn failed_with(error: impl Into<String>) -> Self {
        Self {
            output: None,
            success: false,
            error: Some(error.into()),
            events_json: None,
        }
    }

    pub fn with_events_json(mut self, events_json: impl Into<String>) -> Self {
        self.events_json = Some(events_json.into());
        self
    }
}

/// Commands the daemon (or web console) can send into a running scheduler.
#[derive(Debug)]
pub enum SchedulerCmd {
    /// Run a job immediately, exactly once, without touching its schedule —
    /// `next_run`, `run_count`, and `repeat` are unaffected; the attempt is
    /// recorded in `cron_runs` only (Jenkins "Build Now" semantics).
    /// `fired_by` is the provenance recorded with the run (`manual`,
    /// `webhook`); `payload` is appended to the job's prompt as a fenced
    /// context block (webhook deliveries, criterion 26).
    Trigger {
        job_id: String,
        fired_by: &'static str,
        payload: Option<String>,
    },
}

/// Cheap clonable handle for sending [`SchedulerCmd`]s into the tick loop.
#[derive(Clone)]
pub struct SchedulerHandle {
    tx: mpsc::Sender<SchedulerCmd>,
}

impl SchedulerHandle {
    /// Request an immediate manual run. Returns `false` if the scheduler is gone.
    pub async fn trigger(&self, job_id: impl Into<String>) -> bool {
        self.tx
            .send(SchedulerCmd::Trigger {
                job_id: job_id.into(),
                fired_by: "manual",
                payload: None,
            })
            .await
            .is_ok()
    }

    /// Webhook delivery (criterion 26): immediate run with the request body
    /// exposed to the job as a fenced context block. Returns `false` if the
    /// scheduler is gone.
    pub async fn trigger_webhook(&self, job_id: impl Into<String>, body: Option<String>) -> bool {
        self.tx
            .send(SchedulerCmd::Trigger {
                job_id: job_id.into(),
                fired_by: "webhook",
                payload: body.map(|b| format!("## Webhook payload\n```json\n{b}\n```")),
            })
            .await
            .is_ok()
    }
}

/// Executes a single due job. Implemented by the binary, where the gateway
/// context and delivery sinks live; the scheduler only orchestrates timing.
pub trait JobRunner: Send + Sync {
    fn run(&self, job: CronJob) -> Pin<Box<dyn Future<Output = JobOutcome> + Send + '_>>;
}

/// The cron engine. Cheap to clone-by-reference via `Arc` collaborators.
pub struct Scheduler {
    store: CronStore,
    runner: Arc<dyn JobRunner>,
    tick: Duration,
    inflight: Arc<Mutex<HashSet<String>>>,
    /// Optional per-run history writer (web console). `None` keeps Phase-6
    /// behavior byte-identical: no `cron_runs` rows are ever written.
    runs: Option<RunStore>,
    /// Optional live event feed (web console SSE). Send failures are ignored —
    /// no subscriber is a normal state, never an error.
    events: Option<broadcast::Sender<RunEvent>>,
    /// Command channel. The scheduler keeps one sender alive so `recv` can
    /// never observe a closed channel and busy-loop the `select!`.
    cmd_tx: mpsc::Sender<SchedulerCmd>,
    cmd_rx: Mutex<mpsc::Receiver<SchedulerCmd>>,
    /// Retention (criterion 31): prune `cron_runs` older than this many days
    /// on a daily cadence. `None` = never prune (historical behavior).
    retention_days: Option<i64>,
    last_prune: Mutex<Option<std::time::Instant>>,
}

const DEFAULT_TICK_SECS: u64 = 30;
const CMD_CHANNEL_CAP: usize = 32;

impl Scheduler {
    pub fn new(store: CronStore, runner: Arc<dyn JobRunner>) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(CMD_CHANNEL_CAP);
        Self {
            store,
            runner,
            tick: Duration::from_secs(DEFAULT_TICK_SECS),
            inflight: Arc::new(Mutex::new(HashSet::new())),
            runs: None,
            events: None,
            cmd_tx,
            cmd_rx: Mutex::new(cmd_rx),
            retention_days: None,
            last_prune: Mutex::new(None),
        }
    }

    /// Enable daily pruning of run history older than `days` (criterion 31).
    pub fn with_retention_days(mut self, days: i64) -> Self {
        self.retention_days = Some(days.max(0));
        self
    }

    /// Override the tick interval (config `[scheduler] tick_secs`).
    pub fn with_tick(mut self, tick: Duration) -> Self {
        self.tick = tick;
        self
    }

    /// Record every execution attempt in `cron_runs` (web console history).
    pub fn with_run_store(mut self, runs: RunStore) -> Self {
        self.runs = Some(runs);
        self
    }

    /// Publish run lifecycle events to the web console SSE feed.
    pub fn with_events(mut self, events: broadcast::Sender<RunEvent>) -> Self {
        self.events = Some(events);
        self
    }

    /// A clonable handle for sending commands (e.g. manual triggers) into the
    /// tick loop while it runs.
    pub fn handle(&self) -> SchedulerHandle {
        SchedulerHandle {
            tx: self.cmd_tx.clone(),
        }
    }

    /// Run the tick loop until `cancel` fires. Each due job is spawned into
    /// `tracker`; the caller drains the tracker after cancelling to let
    /// in-flight jobs finish (bounded grace).
    pub async fn run(&self, cancel: CancellationToken, tracker: TaskTracker) {
        let mut ticker = tokio::time::interval(self.tick);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        // The first tick fires immediately; consume it so we don't double-dispatch
        // at startup before the loop proper begins.
        ticker.tick().await;

        tracing::info!(tick_secs = self.tick.as_secs(), "scheduler started");
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    tracing::info!("scheduler stopping (cancelled)");
                    break;
                }
                cmd = Self::recv_cmd(&self.cmd_rx) => {
                    match cmd {
                        Some(SchedulerCmd::Trigger { job_id, fired_by, payload }) => {
                            self.dispatch_trigger_as(&job_id, &tracker, fired_by, payload).await;
                        }
                        // Unreachable while `self.cmd_tx` is alive; guard anyway.
                        None => {}
                    }
                }
                _ = ticker.tick() => {
                    self.dispatch_due(Utc::now(), &tracker).await;
                    self.maybe_prune().await;
                }
            }
        }
    }

    /// Receive one command. `mpsc::Receiver::recv` is cancel-safe, so losing
    /// the `select!` race cannot drop a queued command.
    async fn recv_cmd(rx: &Mutex<mpsc::Receiver<SchedulerCmd>>) -> Option<SchedulerCmd> {
        rx.lock().await.recv().await
    }

    /// Daily retention sweep (criterion 31): at most once per 24h, delete
    /// runs older than `retention_days`. No-op without a run store or policy.
    async fn maybe_prune(&self) {
        let (Some(days), Some(runs)) = (self.retention_days, &self.runs) else {
            return;
        };
        {
            let last = self.last_prune.lock().await;
            if let Some(at) = *last
                && at.elapsed() < Duration::from_secs(24 * 60 * 60)
            {
                return;
            }
        }
        self.prune_now(days, runs).await;
        *self.last_prune.lock().await = Some(std::time::Instant::now());
    }

    /// The actual sweep — separated so tests can exercise it directly.
    async fn prune_now(&self, days: i64, runs: &RunStore) {
        let cutoff = Utc::now() - chrono::Duration::days(days);
        match runs.prune_before(cutoff).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(pruned = n, retention_days = days, "run history pruned"),
            Err(e) => tracing::error!(error = %e, "run-history prune failed"),
        }
    }

    /// Select jobs due at `now` and spawn each that isn't already running.
    /// Exposed within the crate so timing logic is unit-testable with a fixed
    /// `now` rather than wall-clock.
    pub async fn dispatch_due(&self, now: DateTime<Utc>, tracker: &TaskTracker) {
        let due = match self.store.due(now).await {
            Ok(jobs) => jobs,
            Err(e) => {
                tracing::error!(error = %e, "cron due-query failed");
                return;
            }
        };

        for job in due {
            self.spawn_job(job, now, tracker, true, "cron").await;
        }
    }

    /// Run one job immediately (manual "Build Now"). The schedule is not
    /// advanced: `mark_run` is skipped, so `next_run`/`run_count`/`repeat`
    /// are untouched; only `cron_runs` records the attempt.
    pub async fn dispatch_trigger(&self, job_id: &str, tracker: &TaskTracker) {
        self.dispatch_trigger_as(job_id, tracker, "manual", None)
            .await;
    }

    /// Immediate run with provenance and an optional context payload
    /// (criterion 26). The payload reaches the agent as a fenced block under
    /// the job's own prompt — no trait or schema change.
    pub async fn dispatch_trigger_as(
        &self,
        job_id: &str,
        tracker: &TaskTracker,
        fired_by: &'static str,
        payload: Option<String>,
    ) {
        let mut job = match self.store.get(job_id).await {
            Ok(Some(job)) => job,
            Ok(None) => {
                tracing::warn!(job = %job_id, "trigger for unknown job ignored");
                return;
            }
            Err(e) => {
                tracing::error!(job = %job_id, error = %e, "trigger lookup failed");
                return;
            }
        };
        // The payload arrives pre-formatted by the sender (webhook block,
        // failure context) — append as-is under the job's own prompt.
        if let Some(payload) = payload {
            job.prompt = format!("{}\n\n{payload}", job.prompt);
        }
        self.spawn_job(job, Utc::now(), tracker, false, fired_by)
            .await;
    }

    /// Spawn one job into `tracker` unless it is already in flight.
    /// `advance` distinguishes a scheduled fire (advance the schedule via
    /// `mark_run`) from a manual trigger (history row only). `fired_by` is
    /// recorded as the run's provenance (v8).
    async fn spawn_job(
        &self,
        job: CronJob,
        now: DateTime<Utc>,
        tracker: &TaskTracker,
        advance: bool,
        fired_by: &'static str,
    ) {
        // Skip jobs already in flight to avoid double-dispatch across ticks.
        {
            let mut guard = self.inflight.lock().await;
            if !guard.insert(job.id.clone()) {
                return;
            }
        }

        let store = self.store.clone();
        let runs = self.runs.clone();
        let events = self.events.clone();
        let runner = Arc::clone(&self.runner);
        let inflight = Arc::clone(&self.inflight);
        let cmd_tx = self.cmd_tx.clone();
        let job_id = job.id.clone();
        let job_name = job.name.clone().unwrap_or_else(|| job.id.clone());

        tracker.spawn(async move {
            tracing::info!(job = %job_name, manual = !advance, "cron job firing");

            // Retry chain (criterion 28): attempt 1 + up to retry_max retries
            // with exponential backoff. retry_max = 0 keeps the historical
            // single-attempt behavior byte-identical.
            let max_attempts = 1 + job.retry_max.max(0);
            let mut attempt: i64 = 1;
            let outcome = loop {
                // Open the history row before the agent starts; a process
                // crash leaves it `running` — useful forensic signal.
                let run_id = match &runs {
                    Some(rs) => match rs.insert_running_as(&job_id, now, fired_by, attempt).await {
                        Ok(run) => Some(run.id),
                        Err(e) => {
                            tracing::error!(job = %job_name, error = %e, "cron_runs insert failed");
                            None
                        }
                    },
                    None => None,
                };

                if let (Some(ev), Some(run_id)) = (&events, &run_id) {
                    let _ = ev.send(RunEvent::RunStarted {
                        job_id: job_id.clone(),
                        run_id: run_id.clone(),
                    });
                }

                // Correlation span (criterion 17): everything the runner logs
                // — agent events, tool calls, LLM retries — carries both ids.
                let run_span = tracing::info_span!(
                    "run",
                    job_id = %job_id,
                    run_id = %run_id.as_deref().unwrap_or("none"),
                );

                metrics::gauge!("talon_active_jobs").increment(1);
                let started = std::time::Instant::now();
                let outcome = {
                    use tracing::Instrument as _;
                    runner.run(job.clone()).instrument(run_span).await
                };
                metrics::histogram!("talon_run_duration_seconds")
                    .record(started.elapsed().as_secs_f64());
                metrics::gauge!("talon_active_jobs").decrement(1);
                metrics::counter!(
                    "talon_runs_total",
                    "status" => if outcome.success { "success" } else { "failure" },
                )
                .increment(1);

                if let (Some(rs), Some(run_id)) = (&runs, run_id) {
                    let status = if outcome.success {
                        RunStatus::Success
                    } else {
                        RunStatus::Failure
                    };
                    if let Err(e) = rs
                        .finalize(
                            &run_id,
                            status,
                            outcome.output.clone(),
                            outcome.error.clone(),
                            outcome.events_json.clone(),
                        )
                        .await
                    {
                        tracing::error!(job = %job_name, error = %e, "cron_runs finalize failed");
                    }
                    if let Some(ev) = &events {
                        let _ = ev.send(RunEvent::RunFinished {
                            job_id: job_id.clone(),
                            run_id,
                            status,
                        });
                    }
                }

                if outcome.success || attempt >= max_attempts {
                    break outcome;
                }
                // Exponential backoff with jitter: 1s, 2s, 4s … capped at 60s.
                let base = 1u64 << (attempt.min(6) - 1) as u32;
                let jitter_ms = u64::from(uuid::Uuid::new_v4().as_u128() as u16) % 500;
                let delay = Duration::from_secs(base.min(60)) + Duration::from_millis(jitter_ms);
                tracing::warn!(
                    job = %job_name,
                    attempt,
                    max_attempts,
                    delay_secs = delay.as_secs_f32(),
                    "run failed — retrying"
                );
                tokio::time::sleep(delay).await;
                attempt += 1;
            };

            // Advance the schedule from the fire instant (`now`), not from
            // whenever the agent finished — keeps a slow job from dragging
            // its own cadence forward and makes dispatch deterministic.
            if advance && let Err(e) = store.mark_run(&job_id, now, outcome.output.clone()).await {
                tracing::error!(job = %job_name, error = %e, "mark_run failed");
            }

            // Error handler (criterion 29): fire once after the FINAL failed
            // attempt. Handler runs carry fired_by = "failure" and this guard
            // skips on_failure for them — handlers never cascade.
            if !outcome.success
                && fired_by != "failure"
                && let Some(handler_id) = &job.on_failure
            {
                let context = format!(
                    "## Failure context\nsource job: {job_id} ({job_name})\nerror: {}",
                    outcome.error.as_deref().unwrap_or("(no error message)")
                );
                tracing::warn!(
                    job = %job_name,
                    handler = %handler_id,
                    "final attempt failed — triggering on_failure handler"
                );
                let _ = cmd_tx
                    .send(SchedulerCmd::Trigger {
                        job_id: handler_id.clone(),
                        fired_by: "failure",
                        payload: Some(context),
                    })
                    .await;
            }

            inflight.lock().await.remove(&job_id);
        });
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use talon_memory::{CronSchedule, Database};

    use super::*;

    /// A runner that counts invocations and returns a fixed output.
    struct CountingRunner {
        calls: Arc<AtomicUsize>,
    }

    impl JobRunner for CountingRunner {
        fn run(&self, _job: CronJob) -> Pin<Box<dyn Future<Output = JobOutcome> + Send + '_>> {
            let calls = Arc::clone(&self.calls);
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                JobOutcome::ok(Some("done".to_string()))
            })
        }
    }

    /// A runner that always fails with an error message.
    struct FailingRunner;

    impl JobRunner for FailingRunner {
        fn run(&self, _job: CronJob) -> Pin<Box<dyn Future<Output = JobOutcome> + Send + '_>> {
            Box::pin(async move { JobOutcome::failed_with("provider exploded") })
        }
    }

    /// A runner that counts invocations and holds each run open for `hold`.
    struct SlowRunner {
        calls: Arc<AtomicUsize>,
        hold: Duration,
    }

    impl JobRunner for SlowRunner {
        fn run(&self, _job: CronJob) -> Pin<Box<dyn Future<Output = JobOutcome> + Send + '_>> {
            let calls = Arc::clone(&self.calls);
            let hold = self.hold;
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(hold).await;
                JobOutcome::ok(None)
            })
        }
    }

    async fn store() -> CronStore {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        CronStore::new(db)
    }

    async fn stores() -> (CronStore, RunStore) {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        (CronStore::new(Arc::clone(&db)), RunStore::new(db))
    }

    fn counting() -> (Arc<AtomicUsize>, Arc<dyn JobRunner>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let runner: Arc<dyn JobRunner> = Arc::new(CountingRunner {
            calls: Arc::clone(&calls),
        });
        (calls, runner)
    }

    async fn drain(tracker: TaskTracker) {
        tracker.close();
        tracker.wait().await;
    }

    /// Shared-buffer writer so a test can capture formatted tracing output.
    #[derive(Clone)]
    struct BufWriter(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for BufWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("lock").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufWriter {
        type Writer = BufWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Fails until the `succeed_on`-th invocation (criterion 28).
    struct FlakyRunner {
        calls: Arc<AtomicUsize>,
        succeed_on: usize,
    }

    impl JobRunner for FlakyRunner {
        fn run(&self, _job: CronJob) -> Pin<Box<dyn Future<Output = JobOutcome> + Send + '_>> {
            let calls = Arc::clone(&self.calls);
            let succeed_on = self.succeed_on;
            Box::pin(async move {
                let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                if n >= succeed_on {
                    JobOutcome::ok(Some("recovered".to_string()))
                } else {
                    JobOutcome::failed_with(format!("flaky failure #{n}"))
                }
            })
        }
    }

    /// Always fails, recording which jobs it ran (criterion 29).
    struct RecordingFailRunner {
        seen: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl JobRunner for RecordingFailRunner {
        fn run(&self, job: CronJob) -> Pin<Box<dyn Future<Output = JobOutcome> + Send + '_>> {
            let seen = Arc::clone(&self.seen);
            Box::pin(async move {
                seen.lock().expect("lock").push(job.id.clone());
                JobOutcome::failed_with("always fails")
            })
        }
    }

    /// Criterion 28: retry_max = N retries failed attempts with each attempt
    /// in its own row; the first success stops the chain.
    #[tokio::test(start_paused = true)]
    async fn retry_chain_records_attempts_until_success() {
        let (cron, runs) = stores().await;
        let job = cron
            .create(
                CronJob::new("p", CronSchedule::Cron("0 0 * * *".into()), "s").with_retry_max(3),
            )
            .await
            .expect("job");

        let calls = Arc::new(AtomicUsize::new(0));
        let scheduler = Scheduler::new(
            cron.clone(),
            Arc::new(FlakyRunner {
                calls: Arc::clone(&calls),
                succeed_on: 3,
            }),
        )
        .with_run_store(runs.clone());
        let tracker = TaskTracker::new();
        scheduler.dispatch_trigger(&job.id, &tracker).await;
        drain(tracker).await;

        assert_eq!(calls.load(Ordering::SeqCst), 3, "stopped on first success");
        let rows = runs.list_for_job(&job.id, 10).await.expect("rows");
        assert_eq!(rows.len(), 3, "one row per attempt");
        // Newest first: final attempt succeeded, earlier ones failed.
        assert_eq!(rows[0].attempt, 3);
        assert_eq!(rows[0].status, RunStatus::Success);
        assert_eq!(rows[2].attempt, 1);
        assert_eq!(rows[2].status, RunStatus::Failure);
        assert!(rows.iter().all(|r| r.fired_by == "manual"));
    }

    /// retry_max = 0 keeps the historical single-attempt behavior.
    #[tokio::test]
    async fn no_retry_by_default() {
        let (cron, runs) = stores().await;
        let job = cron
            .create(CronJob::new(
                "p",
                CronSchedule::Cron("0 0 * * *".into()),
                "s",
            ))
            .await
            .expect("job");

        let scheduler =
            Scheduler::new(cron.clone(), Arc::new(FailingRunner)).with_run_store(runs.clone());
        let tracker = TaskTracker::new();
        scheduler.dispatch_trigger(&job.id, &tracker).await;
        drain(tracker).await;

        let rows = runs.list_for_job(&job.id, 10).await.expect("rows");
        assert_eq!(rows.len(), 1, "single attempt");
        assert_eq!(rows[0].status, RunStatus::Failure);
    }

    /// Criterion 29: the on_failure handler fires once after the final failed
    /// attempt, and a failing handler never cascades to its own handler.
    #[tokio::test]
    async fn failure_handler_fires_once_and_never_cascades() {
        let (cron, runs) = stores().await;
        // c <- b's handler, b <- a's handler; everything fails.
        let c = cron
            .create(CronJob::new(
                "c",
                CronSchedule::Cron("0 0 * * *".into()),
                "s",
            ))
            .await
            .expect("c");
        let b = cron
            .create(
                CronJob::new("b", CronSchedule::Cron("0 0 * * *".into()), "s")
                    .with_on_failure(Some(c.id.clone())),
            )
            .await
            .expect("b");
        let a = cron
            .create(
                CronJob::new("a", CronSchedule::Cron("0 0 * * *".into()), "s")
                    .with_on_failure(Some(b.id.clone())),
            )
            .await
            .expect("a");

        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let scheduler = Arc::new(
            Scheduler::new(
                cron.clone(),
                Arc::new(RecordingFailRunner {
                    seen: Arc::clone(&seen),
                }),
            )
            .with_run_store(runs.clone())
            .with_tick(Duration::from_secs(3600)),
        );

        // The handler trigger flows through the command channel, so the tick
        // loop must be live.
        let cancel = CancellationToken::new();
        let tracker = TaskTracker::new();
        let loop_sched = Arc::clone(&scheduler);
        let loop_cancel = cancel.clone();
        let loop_tracker = tracker.clone();
        let run_loop = tokio::spawn(async move { loop_sched.run(loop_cancel, loop_tracker).await });

        assert!(scheduler.handle().trigger(&a.id).await);
        // Wait until the handler (b) has run; c must never run.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if seen.lock().expect("lock").len() >= 2 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "handler did not fire in time: {:?}",
                seen.lock().expect("lock")
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // Give a would-be cascade a moment to (incorrectly) appear.
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();
        let _ = run_loop.await;
        drain(tracker).await;

        let seen = seen.lock().expect("lock").clone();
        assert_eq!(
            seen,
            vec![a.id.clone(), b.id.clone()],
            "a then handler b, never c"
        );

        let b_rows = runs.list_for_job(&b.id, 10).await.expect("rows");
        assert_eq!(b_rows[0].fired_by, "failure", "handler provenance");
        assert!(
            runs.list_for_job(&c.id, 10).await.expect("rows").is_empty(),
            "no cascade"
        );
    }

    /// Criterion 31: the retention sweep deletes old completed runs and keeps
    /// recent ones; without a policy nothing is touched.
    #[tokio::test]
    async fn retention_prunes_old_completed_runs() {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        let cron = CronStore::new(Arc::clone(&db));
        let runs = RunStore::new(Arc::clone(&db));
        let job = cron
            .create(CronJob::new(
                "p",
                CronSchedule::Cron("0 0 * * *".into()),
                "s",
            ))
            .await
            .expect("job");

        let old = runs
            .insert_running(&job.id, Utc::now())
            .await
            .expect("old run");
        runs.finalize(&old.id, RunStatus::Success, None, None, None)
            .await
            .expect("finalize old");
        let recent = runs
            .insert_running(&job.id, Utc::now())
            .await
            .expect("recent run");
        runs.finalize(&recent.id, RunStatus::Success, None, None, None)
            .await
            .expect("finalize recent");

        // Backdate the first run far past any retention window.
        let old_id = old.id.clone();
        db.pool()
            .get()
            .await
            .expect("conn")
            .interact(move |conn| {
                conn.execute(
                    "UPDATE cron_runs SET started_at='2020-01-01T00:00:00Z' WHERE id=?1",
                    [old_id],
                )
            })
            .await
            .expect("interact")
            .expect("backdate");

        let (_, counting_runner) = counting();
        let scheduler = Scheduler::new(cron.clone(), counting_runner)
            .with_run_store(runs.clone())
            .with_retention_days(30);
        scheduler.maybe_prune().await;

        let remaining = runs.list_for_job(&job.id, 10).await.expect("rows");
        assert_eq!(remaining.len(), 1, "old pruned, recent kept");
        assert_eq!(remaining[0].id, recent.id);

        // Second sweep inside 24h is a no-op (and must not error).
        scheduler.maybe_prune().await;
        assert_eq!(runs.list_for_job(&job.id, 10).await.expect("rows").len(), 1);
    }

    /// A runner that emits a log line from inside the run.
    struct LoggingRunner;

    impl JobRunner for LoggingRunner {
        fn run(&self, _job: CronJob) -> Pin<Box<dyn Future<Output = JobOutcome> + Send + '_>> {
            Box::pin(async {
                tracing::info!("inside the runner");
                JobOutcome::ok(None)
            })
        }
    }

    /// Criterion 17: a log line emitted inside a run carries both ids via the
    /// `run` span. Relies on the current-thread test runtime so the
    /// thread-default subscriber covers the spawned run task.
    #[tokio::test]
    async fn run_span_correlates_job_and_run_ids() {
        let (cron, runs) = stores().await;
        let job = cron
            .create(CronJob::new(
                "p",
                CronSchedule::Cron("0 0 * * *".into()),
                "s",
            ))
            .await
            .expect("job");

        let buf = Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(BufWriter(Arc::clone(&buf)))
            .with_ansi(false)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let scheduler =
            Scheduler::new(cron.clone(), Arc::new(LoggingRunner)).with_run_store(runs.clone());
        let tracker = TaskTracker::new();
        scheduler.dispatch_trigger(&job.id, &tracker).await;
        drain(tracker).await;

        let log = String::from_utf8(buf.lock().expect("lock").clone()).expect("utf8");
        let rows = runs.list_for_job(&job.id, 10).await.expect("rows");
        assert_eq!(rows.len(), 1);
        assert!(
            log.contains("inside the runner"),
            "runner line captured: {log}"
        );
        assert!(
            log.contains(&format!("job_id={}", job.id)),
            "job_id in span: {log}"
        );
        assert!(
            log.contains(&format!("run_id={}", rows[0].id)),
            "run_id in span: {log}"
        );
    }

    #[tokio::test]
    async fn dispatch_runs_due_job_and_marks_run() {
        let store = store().await;
        let job = store
            .create(CronJob::new(
                "tick",
                CronSchedule::Cron("* * * * *".into()),
                "s",
            ))
            .await
            .expect("create");

        let (calls, runner) = counting();
        let scheduler = Scheduler::new(store.clone(), runner);
        let tracker = TaskTracker::new();

        // 2 minutes ahead → the minutely job is due.
        let now = Utc::now() + chrono::Duration::minutes(2);
        scheduler.dispatch_due(now, &tracker).await;
        drain(tracker).await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let after = store.get(&job.id).await.expect("get").expect("present");
        assert_eq!(after.run_count, 1);
        assert_eq!(after.last_output.as_deref(), Some("done"));
        assert!(after.last_run.is_some());
    }

    #[tokio::test]
    async fn dispatch_skips_jobs_not_yet_due() {
        let store = store().await;
        store
            .create(CronJob::new(
                "future",
                CronSchedule::Cron("0 0 * * *".into()),
                "s",
            ))
            .await
            .expect("create");

        let (calls, runner) = counting();
        let scheduler = Scheduler::new(store, runner);
        let tracker = TaskTracker::new();

        // 1 minute ahead: a daily-midnight job is (almost certainly) not due.
        let now = Utc::now() + chrono::Duration::minutes(1);
        scheduler.dispatch_due(now, &tracker).await;
        drain(tracker).await;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn no_backfill_second_dispatch_at_same_now_does_not_refire() {
        let store = store().await;
        let job = store
            .create(CronJob::new(
                "tick",
                CronSchedule::Cron("* * * * *".into()),
                "s",
            ))
            .await
            .expect("create");

        let (calls, runner) = counting();
        let scheduler = Scheduler::new(store.clone(), runner);

        let now = Utc::now() + chrono::Duration::minutes(2);

        let t1 = TaskTracker::new();
        scheduler.dispatch_due(now, &t1).await;
        drain(t1).await;

        // next_run is now recomputed forward past `now`; a second pass is a no-op.
        let t2 = TaskTracker::new();
        scheduler.dispatch_due(now, &t2).await;
        drain(t2).await;

        assert_eq!(calls.load(Ordering::SeqCst), 1, "must not backfill/refire");
        let after = store.get(&job.id).await.expect("get").expect("present");
        assert_eq!(after.run_count, 1);
    }

    #[tokio::test]
    async fn disabled_job_never_dispatches() {
        let store = store().await;
        let job = store
            .create(CronJob::new(
                "tick",
                CronSchedule::Cron("* * * * *".into()),
                "s",
            ))
            .await
            .expect("create");
        store.set_enabled(&job.id, false).await.expect("disable");

        let (calls, runner) = counting();
        let scheduler = Scheduler::new(store, runner);
        let tracker = TaskTracker::new();
        scheduler
            .dispatch_due(Utc::now() + chrono::Duration::minutes(2), &tracker)
            .await;
        drain(tracker).await;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn run_loop_fires_a_due_job_on_its_own_timer() {
        // 6.11 live-fire proof: the real `run()` tick loop (not a manual
        // `dispatch_due` call) must pick up a job once it comes due and invoke
        // the runner exactly once. We use a one-shot due ~1s out so the test is
        // deterministic and fast; minutely *due-detection* is covered separately
        // by `due_picks_up_minutely_cron_job` / `dispatch_runs_due_job`.
        // (`next_run` is stored at whole-second precision, so the due instant
        // must be a whole second in the future — sub-second offsets floor away.)
        let store = store().await;
        let soon = Utc::now().timestamp() + 1;
        let job = store
            .create(CronJob::new("soon", CronSchedule::Once(soon), "s"))
            .await
            .expect("create");

        let (calls, runner) = counting();
        let scheduler = Scheduler::new(store.clone(), runner).with_tick(Duration::from_millis(50));
        let cancel = CancellationToken::new();
        let tracker = TaskTracker::new();

        let handle = {
            let cancel = cancel.clone();
            let tracker = tracker.clone();
            tokio::spawn(async move { scheduler.run(cancel, tracker).await })
        };

        // Give the loop time to cross the due instant (~1s) and fire several ticks past it.
        tokio::time::sleep(Duration::from_millis(1800)).await;
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("loop did not exit on cancel")
            .expect("join");
        drain(tracker).await;

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "loop must fire the due job once"
        );
        let after = store.get(&job.id).await.expect("get").expect("present");
        assert_eq!(after.run_count, 1);
        assert!(
            after.next_run.is_none(),
            "one-shot clears next_run after firing"
        );
    }

    #[tokio::test]
    async fn dispatch_with_run_store_records_success_lifecycle() {
        let (store, runs) = stores().await;
        let job = store
            .create(CronJob::new(
                "tick",
                CronSchedule::Cron("* * * * *".into()),
                "s",
            ))
            .await
            .expect("create");

        let (_calls, runner) = counting();
        let scheduler = Scheduler::new(store.clone(), runner).with_run_store(runs.clone());
        let tracker = TaskTracker::new();
        scheduler
            .dispatch_due(Utc::now() + chrono::Duration::minutes(2), &tracker)
            .await;
        drain(tracker).await;

        let history = runs.list_for_job(&job.id, 10).await.expect("history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, talon_memory::RunStatus::Success);
        assert_eq!(history[0].output.as_deref(), Some("done"));
        assert!(history[0].finished_at.is_some());
    }

    #[tokio::test]
    async fn dispatch_with_run_store_records_failure_and_keeps_mark_run_semantics() {
        let (store, runs) = stores().await;
        let job = store
            .create(CronJob::new(
                "tick",
                CronSchedule::Cron("* * * * *".into()),
                "s",
            ))
            .await
            .expect("create");

        let runner: Arc<dyn JobRunner> = Arc::new(FailingRunner);
        let scheduler = Scheduler::new(store.clone(), runner).with_run_store(runs.clone());
        let tracker = TaskTracker::new();
        scheduler
            .dispatch_due(Utc::now() + chrono::Duration::minutes(2), &tracker)
            .await;
        drain(tracker).await;

        let history = runs.list_for_job(&job.id, 10).await.expect("history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, talon_memory::RunStatus::Failure);
        assert_eq!(history[0].error.as_deref(), Some("provider exploded"));
        assert!(history[0].output.is_none());

        // Existing completion semantics unchanged: mark_run still ran (schedule
        // advanced, run_count bumped) — only a process crash re-fires.
        let after = store.get(&job.id).await.expect("get").expect("present");
        assert_eq!(after.run_count, 1);
        assert!(after.last_output.is_none(), "failure stores no output");
    }

    #[tokio::test]
    async fn without_run_store_no_history_is_written() {
        let (store, runs) = stores().await;
        let job = store
            .create(CronJob::new(
                "tick",
                CronSchedule::Cron("* * * * *".into()),
                "s",
            ))
            .await
            .expect("create");

        let (calls, runner) = counting();
        let scheduler = Scheduler::new(store.clone(), runner); // no run store
        let tracker = TaskTracker::new();
        scheduler
            .dispatch_due(Utc::now() + chrono::Duration::minutes(2), &tracker)
            .await;
        drain(tracker).await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            runs.list_for_job(&job.id, 10)
                .await
                .expect("history")
                .is_empty(),
            "Phase-6 behavior byte-identical without with_run_store"
        );
    }

    #[tokio::test]
    async fn trigger_runs_once_without_advancing_schedule() {
        let (store, runs) = stores().await;
        // A far-future daily job — never due on its own.
        let job = store
            .create(CronJob::new(
                "manual",
                CronSchedule::Cron("0 0 * * *".into()),
                "s",
            ))
            .await
            .expect("create");
        let before = store.get(&job.id).await.expect("get").expect("present");

        let (calls, runner) = counting();
        let scheduler = Scheduler::new(store.clone(), runner).with_run_store(runs.clone());
        let tracker = TaskTracker::new();
        scheduler.dispatch_trigger(&job.id, &tracker).await;
        drain(tracker).await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Schedule untouched: next_run, run_count, last_run all unchanged.
        let after = store.get(&job.id).await.expect("get").expect("present");
        assert_eq!(after.next_run, before.next_run);
        assert_eq!(after.run_count, 0);
        assert!(after.last_run.is_none());

        // But the attempt is in the history.
        let history = runs.list_for_job(&job.id, 10).await.expect("history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, talon_memory::RunStatus::Success);
    }

    #[tokio::test]
    async fn trigger_while_inflight_is_noop() {
        let (store, runs) = stores().await;
        let job = store
            .create(CronJob::new(
                "slow",
                CronSchedule::Cron("0 0 * * *".into()),
                "s",
            ))
            .await
            .expect("create");

        let calls = Arc::new(AtomicUsize::new(0));
        let runner: Arc<dyn JobRunner> = Arc::new(SlowRunner {
            calls: Arc::clone(&calls),
            hold: Duration::from_millis(300),
        });
        let scheduler = Scheduler::new(store.clone(), runner).with_run_store(runs.clone());
        let tracker = TaskTracker::new();

        scheduler.dispatch_trigger(&job.id, &tracker).await;
        // Give the first spawn a moment to take the inflight slot.
        tokio::time::sleep(Duration::from_millis(50)).await;
        scheduler.dispatch_trigger(&job.id, &tracker).await;
        drain(tracker).await;

        assert_eq!(calls.load(Ordering::SeqCst), 1, "second trigger is a no-op");
        assert_eq!(
            runs.list_for_job(&job.id, 10).await.expect("history").len(),
            1
        );
    }

    #[tokio::test]
    async fn events_emitted_in_lifecycle_order() {
        let (store, runs) = stores().await;
        let job = store
            .create(CronJob::new(
                "evented",
                CronSchedule::Cron("0 0 * * *".into()),
                "s",
            ))
            .await
            .expect("create");

        let (ev_tx, mut ev_rx) = broadcast::channel::<RunEvent>(16);
        let (_calls, runner) = counting();
        let scheduler = Scheduler::new(store, runner)
            .with_run_store(runs)
            .with_events(ev_tx);
        let tracker = TaskTracker::new();
        scheduler.dispatch_trigger(&job.id, &tracker).await;
        drain(tracker).await;

        let first = ev_rx.try_recv().expect("started event");
        match first {
            RunEvent::RunStarted { job_id, .. } => assert_eq!(job_id, job.id),
            other => panic!("expected RunStarted, got {other:?}"),
        }
        let second = ev_rx.try_recv().expect("finished event");
        match second {
            RunEvent::RunFinished { job_id, status, .. } => {
                assert_eq!(job_id, job.id);
                assert_eq!(status, RunStatus::Success);
            }
            other => panic!("expected RunFinished, got {other:?}"),
        }
    }

    #[test]
    fn run_event_serializes_with_type_tag() {
        let ev = RunEvent::RunFinished {
            job_id: "j1".into(),
            run_id: "r1".into(),
            status: RunStatus::Failure,
        };
        let json = serde_json::to_value(&ev).expect("serialize");
        assert_eq!(json["type"], "run_finished");
        assert_eq!(json["status"], "failure");
    }

    #[tokio::test]
    async fn trigger_unknown_job_is_ignored() {
        let (store, _runs) = stores().await;
        let (calls, runner) = counting();
        let scheduler = Scheduler::new(store, runner);
        let tracker = TaskTracker::new();
        scheduler.dispatch_trigger("no-such-job", &tracker).await;
        drain(tracker).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn handle_trigger_reaches_running_loop() {
        let (store, runs) = stores().await;
        let job = store
            .create(CronJob::new(
                "via-handle",
                CronSchedule::Cron("0 0 * * *".into()),
                "s",
            ))
            .await
            .expect("create");

        let (calls, runner) = counting();
        let scheduler = Scheduler::new(store.clone(), runner).with_run_store(runs.clone());
        let handle = scheduler.handle();
        let cancel = CancellationToken::new();
        let tracker = TaskTracker::new();

        let loop_handle = {
            let cancel = cancel.clone();
            let tracker = tracker.clone();
            tokio::spawn(async move { scheduler.run(cancel, tracker).await })
        };

        assert!(handle.trigger(&job.id).await, "send into live loop");
        // Let the loop pick the command up and spawn the job.
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), loop_handle)
            .await
            .expect("loop did not exit on cancel")
            .expect("join");
        drain(tracker).await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            runs.list_for_job(&job.id, 10).await.expect("history").len(),
            1
        );
    }

    #[tokio::test]
    async fn run_loop_exits_promptly_on_cancel() {
        let store = store().await;
        let (_calls, runner) = counting();
        let scheduler = Scheduler::new(store, runner).with_tick(Duration::from_millis(20));
        let cancel = CancellationToken::new();
        let tracker = TaskTracker::new();

        let handle = {
            let cancel = cancel.clone();
            let tracker = tracker.clone();
            tokio::spawn(async move { scheduler.run(cancel, tracker).await })
        };

        // Let a few ticks pass, then cancel.
        tokio::time::sleep(Duration::from_millis(80)).await;
        cancel.cancel();

        // The loop must return quickly after cancellation.
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("loop did not exit on cancel")
            .expect("join");
        drain(tracker).await;
    }
}
