//! In-memory broker for unattended approval escalations (SPEC §4.4 path A).
//!
//! A scheduled job that hits an out-of-scope tool call emits
//! `AgentEvent::ApprovalRequested { tx, .. }` — the `oneshot::Sender<bool>` is
//! an in-process handle and cannot be serialized, so the broker holds it here
//! until the web console resolves it (✅/❌) or a timeout denies it. Pending
//! approvals deliberately die with the daemon: a restart drops them, which is
//! identical to today's Telegram async-approval semantics.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::oneshot;

/// Metadata about one pending approval, safe to serialize to the console.
#[derive(Debug, Clone, Serialize)]
pub struct PendingApproval {
    pub call_id: String,
    /// The cron job that triggered the escalation, when known.
    pub job_id: Option<String>,
    pub tool: String,
    pub args: serde_json::Value,
    /// Unix seconds when the escalation was registered.
    pub requested_at: u64,
}

impl PendingApproval {
    pub fn new(
        call_id: impl Into<String>,
        job_id: Option<String>,
        tool: impl Into<String>,
        args: serde_json::Value,
    ) -> Self {
        Self {
            call_id: call_id.into(),
            job_id,
            tool: tool.into(),
            args,
            requested_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        }
    }
}

struct Pending {
    meta: PendingApproval,
    tx: oneshot::Sender<bool>,
}

/// Clonable registry of pending approvals keyed by `call_id`.
#[derive(Clone, Default)]
pub struct ApprovalBroker {
    inner: Arc<Mutex<HashMap<String, Pending>>>,
}

impl ApprovalBroker {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Pending>> {
        // A poisoned lock only means another thread panicked mid-insert; the
        // map itself is still structurally valid — recover rather than crash
        // the daemon's approval path.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Park an approval until the console (or a timeout) resolves it.
    pub fn register(&self, meta: PendingApproval, tx: oneshot::Sender<bool>) {
        let call_id = meta.call_id.clone();
        self.lock().insert(call_id, Pending { meta, tx });
    }

    /// Resolve a pending approval. Returns `false` if the `call_id` is unknown
    /// (already resolved, timed out, or never registered).
    pub fn resolve(&self, call_id: &str, approve: bool) -> bool {
        match self.lock().remove(call_id) {
            Some(pending) => {
                // The agent side may have given up (run cancelled) — a dead
                // receiver is not an error worth surfacing to the console.
                let _ = pending.tx.send(approve);
                true
            }
            None => false,
        }
    }

    /// Deny an approval if (and only if) it is still pending — the timeout
    /// path. Returns `true` when a deny was actually sent.
    pub fn deny_if_pending(&self, call_id: &str) -> bool {
        self.resolve(call_id, false)
    }

    /// Snapshot of everything currently waiting, oldest first.
    pub fn pending(&self) -> Vec<PendingApproval> {
        let mut all: Vec<PendingApproval> = self.lock().values().map(|p| p.meta.clone()).collect();
        all.sort_by_key(|p| p.requested_at);
        all
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn meta(call_id: &str) -> PendingApproval {
        PendingApproval::new(
            call_id,
            Some("job-1".into()),
            "terminal",
            serde_json::json!({"cmd": "rm -rf /"}),
        )
    }

    #[tokio::test]
    async fn register_then_resolve_approves() {
        let broker = ApprovalBroker::new();
        let (tx, rx) = oneshot::channel();
        broker.register(meta("c1"), tx);

        assert_eq!(broker.pending().len(), 1);
        assert!(broker.resolve("c1", true));
        assert_eq!(rx.await, Ok(true));
        assert!(broker.pending().is_empty());
    }

    #[tokio::test]
    async fn resolve_unknown_call_is_false() {
        let broker = ApprovalBroker::new();
        assert!(!broker.resolve("ghost", true));
    }

    #[tokio::test]
    async fn deny_if_pending_only_fires_once() {
        let broker = ApprovalBroker::new();
        let (tx, rx) = oneshot::channel();
        broker.register(meta("c2"), tx);

        assert!(broker.resolve("c2", true), "console resolves first");
        assert!(!broker.deny_if_pending("c2"), "timeout finds nothing left");
        assert_eq!(rx.await, Ok(true), "approval was not overwritten by deny");
    }

    #[tokio::test]
    async fn pending_sorted_oldest_first() {
        let broker = ApprovalBroker::new();
        let (tx1, _rx1) = oneshot::channel();
        let (tx2, _rx2) = oneshot::channel();
        let mut first = meta("a");
        first.requested_at = 100;
        let mut second = meta("b");
        second.requested_at = 50;
        broker.register(first, tx1);
        broker.register(second, tx2);

        let pending = broker.pending();
        assert_eq!(pending[0].call_id, "b");
        assert_eq!(pending[1].call_id, "a");
    }
}
