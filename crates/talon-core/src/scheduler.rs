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
use tokio::sync::Mutex;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use talon_memory::{CronJob, CronStore};

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
}

impl JobOutcome {
    pub fn ok(output: Option<String>) -> Self {
        Self {
            output,
            success: true,
        }
    }

    pub fn failed() -> Self {
        Self {
            output: None,
            success: false,
        }
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
}

const DEFAULT_TICK_SECS: u64 = 30;

impl Scheduler {
    pub fn new(store: CronStore, runner: Arc<dyn JobRunner>) -> Self {
        Self {
            store,
            runner,
            tick: Duration::from_secs(DEFAULT_TICK_SECS),
            inflight: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Override the tick interval (config `[scheduler] tick_secs`).
    pub fn with_tick(mut self, tick: Duration) -> Self {
        self.tick = tick;
        self
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
                _ = ticker.tick() => {
                    self.dispatch_due(Utc::now(), &tracker).await;
                }
            }
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
            // Skip jobs already in flight to avoid double-dispatch across ticks.
            {
                let mut guard = self.inflight.lock().await;
                if !guard.insert(job.id.clone()) {
                    continue;
                }
            }

            let store = self.store.clone();
            let runner = Arc::clone(&self.runner);
            let inflight = Arc::clone(&self.inflight);
            let job_id = job.id.clone();
            let job_name = job.name.clone().unwrap_or_else(|| job.id.clone());

            tracker.spawn(async move {
                tracing::info!(job = %job_name, "cron job firing");
                let outcome = runner.run(job).await;
                // Advance the schedule from the fire instant (`now`), not from
                // whenever the agent finished — keeps a slow job from dragging
                // its own cadence forward and makes dispatch deterministic.
                if let Err(e) = store.mark_run(&job_id, now, outcome.output).await {
                    tracing::error!(job = %job_name, error = %e, "mark_run failed");
                }
                inflight.lock().await.remove(&job_id);
            });
        }
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

    async fn store() -> CronStore {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        CronStore::new(db)
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
