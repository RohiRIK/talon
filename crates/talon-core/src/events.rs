use serde_json::Value;
use std::fmt;
use tokio::sync::oneshot;

/// Agent lifecycle events emitted on the broadcast channel.
/// Consumers (CLI, TUI, Telegram) subscribe and render accordingly.
///
/// `ApprovalRequested` carries a oneshot sender so each gateway can
/// implement approval in its own way — CLI reads stdin, HTTP returns 202 + webhook, etc.
pub enum AgentEvent {
    Started,
    LlmRequest,
    LlmResponse,
    ToolCalled {
        id: String,
        name: String,
        args: Value,
    },
    ToolResult {
        id: String,
        content: String,
        is_error: bool,
    },
    /// Gateway receives this and sends an approval prompt over its channel.
    /// Reply `true` to allow, `false` to deny.
    ApprovalRequested {
        call_id: String,
        tool_name: String,
        args: Value,
        tx: oneshot::Sender<bool>,
    },
    Completed,
    Failed(String),
}

// oneshot::Sender<bool> does not implement Debug — implement manually.
impl fmt::Debug for AgentEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Started => write!(f, "AgentEvent::Started"),
            Self::LlmRequest => write!(f, "AgentEvent::LlmRequest"),
            Self::LlmResponse => write!(f, "AgentEvent::LlmResponse"),
            Self::ToolCalled { id, name, .. } => f
                .debug_struct("AgentEvent::ToolCalled")
                .field("id", id)
                .field("name", name)
                .finish_non_exhaustive(),
            Self::ToolResult { id, is_error, .. } => f
                .debug_struct("AgentEvent::ToolResult")
                .field("id", id)
                .field("is_error", is_error)
                .finish_non_exhaustive(),
            Self::ApprovalRequested {
                call_id, tool_name, ..
            } => f
                .debug_struct("AgentEvent::ApprovalRequested")
                .field("call_id", call_id)
                .field("tool_name", tool_name)
                .finish_non_exhaustive(),
            Self::Completed => write!(f, "AgentEvent::Completed"),
            Self::Failed(msg) => write!(f, "AgentEvent::Failed({msg:?})"),
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn agent_event_started_debug() {
        let s = format!("{:?}", AgentEvent::Started);
        assert_eq!(s, "AgentEvent::Started");
    }

    #[test]
    fn agent_event_completed_debug() {
        let s = format!("{:?}", AgentEvent::Completed);
        assert_eq!(s, "AgentEvent::Completed");
    }

    #[test]
    fn agent_event_failed_contains_message() {
        let s = format!("{:?}", AgentEvent::Failed("oops".to_string()));
        assert!(s.contains("oops"));
    }

    #[test]
    fn agent_event_tool_called_debug_shows_name() {
        let ev = AgentEvent::ToolCalled {
            id: "t1".to_string(),
            name: "read_file".to_string(),
            args: serde_json::json!({}),
        };
        let s = format!("{ev:?}");
        assert!(s.contains("read_file"));
    }

    #[test]
    fn agent_event_approval_requested_contains_call_id() {
        let (tx, _rx) = oneshot::channel::<bool>();
        let ev = AgentEvent::ApprovalRequested {
            call_id: "call-1".to_string(),
            tool_name: "rm".to_string(),
            args: serde_json::json!({}),
            tx,
        };
        let s = format!("{ev:?}");
        assert!(s.contains("call-1"));
    }

    #[test]
    fn agent_event_llm_request_debug() {
        let s = format!("{:?}", AgentEvent::LlmRequest);
        assert_eq!(s, "AgentEvent::LlmRequest");
    }

    #[test]
    fn agent_event_llm_response_debug() {
        let s = format!("{:?}", AgentEvent::LlmResponse);
        assert_eq!(s, "AgentEvent::LlmResponse");
    }

    #[test]
    fn agent_event_tool_result_debug_shows_id() {
        let ev = AgentEvent::ToolResult {
            id: "r1".to_string(),
            content: "output".to_string(),
            is_error: false,
        };
        let s = format!("{ev:?}");
        assert!(s.contains("r1"));
    }

    #[tokio::test]
    async fn approval_tx_can_send_true() {
        let (tx, rx) = oneshot::channel::<bool>();
        let ev = AgentEvent::ApprovalRequested {
            call_id: "c".to_string(),
            tool_name: "t".to_string(),
            args: serde_json::json!({}),
            tx,
        };
        // Extract tx and send approval
        if let AgentEvent::ApprovalRequested { tx, .. } = ev {
            tx.send(true).expect("send approval");
        }
        let approved = rx.await.expect("receive");
        assert!(approved);
    }

    #[tokio::test]
    async fn approval_tx_can_deny() {
        let (tx, rx) = oneshot::channel::<bool>();
        let ev = AgentEvent::ApprovalRequested {
            call_id: "c2".to_string(),
            tool_name: "rm_rf".to_string(),
            args: serde_json::json!({}),
            tx,
        };
        if let AgentEvent::ApprovalRequested { tx, .. } = ev {
            tx.send(false).expect("send denial");
        }
        let approved = rx.await.expect("receive");
        assert!(!approved);
    }
}
