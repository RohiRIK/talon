use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::json;
use tokio::sync::mpsc;

use talon_core::approval::ApprovalLevel;
use talon_core::tools::{Tool, ToolContext, ToolResult};

/// Callback interface for gateways to receive outbound messages from the agent.
pub trait MessageSink: Send + Sync {
    fn send(&self, channel_id: &str, content: &str) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// A `MessageSink` backed by a `tokio::sync::mpsc` channel.
pub struct ChannelSink {
    tx: mpsc::Sender<(String, String)>,
}

impl ChannelSink {
    pub fn new(tx: mpsc::Sender<(String, String)>) -> Self {
        Self { tx }
    }
}

impl MessageSink for ChannelSink {
    fn send(&self, channel_id: &str, content: &str) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let tx = self.tx.clone();
        let channel = channel_id.to_string();
        let msg = content.to_string();
        Box::pin(async move {
            tx.send((channel, msg)).await.ok();
        })
    }
}

/// Tool that allows the agent to push a message to any registered gateway channel.
///
/// Approval level: `NeedsApproval` — the agent must not send messages without
/// the user seeing what will be sent.
pub struct SendMessageTool {
    sink: Arc<dyn MessageSink>,
}

impl SendMessageTool {
    pub fn new(sink: Arc<dyn MessageSink>) -> Self {
        Self { sink }
    }
}

impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "name": "send_message",
            "description": "Send a message to a specific channel (e.g. telegram, cli, http). \
                            Use this to proactively push updates or notifications to the user.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "channel_id": {
                        "type": "string",
                        "description": "The channel to send to (e.g. 'telegram', 'cli')."
                    },
                    "content": {
                        "type": "string",
                        "description": "The message content to send."
                    }
                },
                "required": ["channel_id", "content"]
            }
        })
    }

    fn approval_level(&self, _args: &serde_json::Value) -> ApprovalLevel {
        ApprovalLevel::NeedsApproval
    }

    fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let sink = Arc::clone(&self.sink);
        Box::pin(async move {
            let channel_id = match args["channel_id"].as_str() {
                Some(c) if !c.is_empty() => c.to_string(),
                _ => return ToolResult::err("Missing required argument: channel_id"),
            };
            let content = match args["content"].as_str() {
                Some(c) if !c.is_empty() => c.to_string(),
                _ => return ToolResult::err("Missing required argument: content"),
            };

            sink.send(&channel_id, &content).await;
            ToolResult::ok(format!("Message sent to {channel_id}"))
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use talon_core::approval::ApprovalLevel;

    fn make_tool() -> (SendMessageTool, mpsc::Receiver<(String, String)>) {
        let (tx, rx) = mpsc::channel(8);
        let sink = Arc::new(ChannelSink::new(tx));
        (SendMessageTool::new(sink), rx)
    }

    #[test]
    fn name() {
        let (tool, _) = make_tool();
        assert_eq!(tool.name(), "send_message");
    }

    #[test]
    fn approval_level_needs_approval() {
        let (tool, _) = make_tool();
        assert_eq!(tool.approval_level(&json!({})), ApprovalLevel::NeedsApproval);
    }

    #[test]
    fn schema_has_required_fields() {
        let (tool, _) = make_tool();
        let s = tool.schema();
        let required = s["input_schema"]["required"]
            .as_array()
            .expect("required array");
        assert!(required.iter().any(|v| v == "channel_id"));
        assert!(required.iter().any(|v| v == "content"));
    }

    #[tokio::test]
    async fn execute_sends_message() {
        let (tool, mut rx) = make_tool();
        let result = tool
            .execute(
                json!({ "channel_id": "telegram", "content": "hello" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.is_error);

        let (channel, msg) = rx.recv().await.expect("received");
        assert_eq!(channel, "telegram");
        assert_eq!(msg, "hello");
    }

    #[tokio::test]
    async fn execute_errors_on_missing_channel_id() {
        let (tool, _) = make_tool();
        let result = tool
            .execute(json!({ "content": "hello" }), ToolContext::default())
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("channel_id"));
    }

    #[tokio::test]
    async fn execute_errors_on_missing_content() {
        let (tool, _) = make_tool();
        let result = tool
            .execute(json!({ "channel_id": "cli" }), ToolContext::default())
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("content"));
    }
}
