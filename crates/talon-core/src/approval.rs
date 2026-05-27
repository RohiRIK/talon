use tokio::sync::mpsc;

use crate::error::CoreError;
use crate::events::AgentEvent;

/// Per-invocation approval classification. Computed with actual tool arguments,
/// not as a static property of the tool — prevents tools lying about danger level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalLevel {
    Safe,
    NeedsApproval,
    Dangerous,
}

/// Sits between the agent loop and every tool call.
/// Safe and NeedsApproval pass through immediately.
/// Dangerous emits `AgentEvent::ApprovalRequested` and awaits the gateway's reply.
pub struct ApprovalMembrane {
    events: mpsc::Sender<AgentEvent>,
}

impl ApprovalMembrane {
    pub fn new(events: mpsc::Sender<AgentEvent>) -> Self {
        Self { events }
    }

    pub async fn check(
        &self,
        call_id: String,
        tool_name: &str,
        args: &serde_json::Value,
        level: ApprovalLevel,
    ) -> Result<(), CoreError> {
        match level {
            ApprovalLevel::Safe | ApprovalLevel::NeedsApproval => Ok(()),
            ApprovalLevel::Dangerous => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                self.events
                    .send(AgentEvent::ApprovalRequested {
                        call_id,
                        tool_name: tool_name.to_string(),
                        args: args.clone(),
                        tx,
                    })
                    .await
                    .map_err(|_| CoreError::InvalidState("event channel closed".to_string()))?;

                let approved = rx
                    .await
                    .map_err(|_| CoreError::InvalidState("approval sender dropped".to_string()))?;

                if approved {
                    Ok(())
                } else {
                    Err(CoreError::ApprovalDenied {
                        tool: tool_name.to_string(),
                    })
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn approval_level_equality() {
        assert_eq!(ApprovalLevel::Safe, ApprovalLevel::Safe);
        assert_ne!(ApprovalLevel::Safe, ApprovalLevel::Dangerous);
    }

    #[test]
    fn approval_level_copy() {
        let a = ApprovalLevel::NeedsApproval;
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn approval_level_debug() {
        assert_eq!(format!("{:?}", ApprovalLevel::Dangerous), "Dangerous");
    }

    #[test]
    fn all_three_variants_are_distinct() {
        assert_ne!(ApprovalLevel::Safe, ApprovalLevel::NeedsApproval);
        assert_ne!(ApprovalLevel::NeedsApproval, ApprovalLevel::Dangerous);
        assert_ne!(ApprovalLevel::Safe, ApprovalLevel::Dangerous);
    }

    #[test]
    fn pattern_match_safe() {
        let level = ApprovalLevel::Safe;
        let auto_approved = matches!(level, ApprovalLevel::Safe | ApprovalLevel::NeedsApproval);
        assert!(auto_approved);
    }

    #[test]
    fn pattern_match_dangerous_requires_gate() {
        let level = ApprovalLevel::Dangerous;
        let needs_gate = matches!(level, ApprovalLevel::Dangerous);
        assert!(needs_gate);
    }

    #[tokio::test]
    async fn membrane_safe_passes_through() {
        let (tx, _rx) = mpsc::channel(8);
        let membrane = ApprovalMembrane::new(tx);
        let result = membrane
            .check(
                "c1".to_string(),
                "read_file",
                &serde_json::json!({}),
                ApprovalLevel::Safe,
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn membrane_needs_approval_passes_through() {
        let (tx, _rx) = mpsc::channel(8);
        let membrane = ApprovalMembrane::new(tx);
        let result = membrane
            .check(
                "c2".to_string(),
                "write_file",
                &serde_json::json!({}),
                ApprovalLevel::NeedsApproval,
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn membrane_dangerous_approved_returns_ok() {
        let (tx, mut rx) = mpsc::channel(8);
        let membrane = ApprovalMembrane::new(tx);

        // Spawn a task that auto-approves the first ApprovalRequested event.
        tokio::spawn(async move {
            if let Some(AgentEvent::ApprovalRequested { tx, .. }) = rx.recv().await {
                tx.send(true).expect("send approval");
            }
        });

        let result = membrane
            .check(
                "c3".to_string(),
                "delete_file",
                &serde_json::json!({"path": "/tmp/test"}),
                ApprovalLevel::Dangerous,
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn membrane_dangerous_denied_returns_error() {
        let (tx, mut rx) = mpsc::channel(8);
        let membrane = ApprovalMembrane::new(tx);

        tokio::spawn(async move {
            if let Some(AgentEvent::ApprovalRequested { tx, .. }) = rx.recv().await {
                tx.send(false).expect("send denial");
            }
        });

        let result = membrane
            .check(
                "c4".to_string(),
                "rm_rf",
                &serde_json::json!({"path": "/"}),
                ApprovalLevel::Dangerous,
            )
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("rm_rf"));
    }

    #[tokio::test]
    async fn membrane_dangerous_closed_channel_returns_error() {
        let (tx, rx) = mpsc::channel(8);
        // Drop rx so the channel is closed.
        drop(rx);
        let membrane = ApprovalMembrane::new(tx);

        let result = membrane
            .check(
                "c5".to_string(),
                "any_tool",
                &serde_json::json!({}),
                ApprovalLevel::Dangerous,
            )
            .await;
        assert!(result.is_err());
    }
}
