//! Deterministic mock LlmProvider for tests and local development.
//! Gated behind `#[cfg(any(test, feature = "mock"))]` — never compiled into release binaries.

use std::{future::Future, pin::Pin, sync::Mutex};

use crate::{ContentBlock, LlmError, LlmProvider, LlmResponse, Message};

/// Returns responses from a fixed queue in FIFO order.
/// Panics (in tests only) when the queue is exhausted — that's an assertion failure, not silent pass.
pub struct MockProvider {
    responses: Mutex<Vec<LlmResponse>>,
}

impl MockProvider {
    /// Build a provider that returns `responses` in order.
    pub fn new(responses: Vec<LlmResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
        }
    }

    /// Single text response — the common case in tests.
    pub fn text(content: impl Into<String>, stop_reason: impl Into<String>) -> Self {
        Self::new(vec![LlmResponse {
            content: vec![ContentBlock::Text {
                text: content.into(),
            }],
            stop_reason: stop_reason.into(),
        }])
    }

    /// Single tool-use response.
    pub fn tool_use(
        id: impl Into<String>,
        name: impl Into<String>,
        input: serde_json::Value,
    ) -> Self {
        Self::new(vec![LlmResponse {
            content: vec![ContentBlock::ToolUse {
                id: id.into(),
                name: name.into(),
                input,
            }],
            stop_reason: "tool_use".to_string(),
        }])
    }

    fn pop_response(&self) -> Result<LlmResponse, LlmError> {
        let mut queue = self
            .responses
            .lock()
            .map_err(|_| LlmError::InvalidResponse("MockProvider mutex poisoned".to_string()))?;
        if queue.is_empty() {
            return Err(LlmError::InvalidResponse(
                "MockProvider queue exhausted — add more responses to the fixture".to_string(),
            ));
        }
        Ok(queue.remove(0))
    }
}

impl LlmProvider for MockProvider {
    fn complete<'a>(
        &'a self,
        _messages: &'a [Message],
        _tools: &'a [serde_json::Value],
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, LlmError>> + Send + 'a>> {
        let result = self.pop_response();
        Box::pin(async move { result })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn mock_provider_returns_queued_text_response() {
        let p = MockProvider::text("hello from mock", "end_turn");
        let resp = p
            .complete(&[], &[])
            .await
            .expect("should return queued response");
        assert_eq!(resp.stop_reason, "end_turn");
        match &resp.content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "hello from mock"),
            _ => panic!("expected Text block"),
        }
    }

    #[tokio::test]
    async fn mock_provider_returns_tool_use_response() {
        let p = MockProvider::tool_use("t1", "echo", serde_json::json!({"msg": "hi"}));
        let resp = p.complete(&[], &[]).await.expect("response");
        assert_eq!(resp.stop_reason, "tool_use");
        match &resp.content[0] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "t1");
                assert_eq!(name, "echo");
                assert_eq!(input["msg"], "hi");
            }
            _ => panic!("expected ToolUse block"),
        }
    }

    #[tokio::test]
    async fn mock_provider_returns_multiple_responses_in_order() {
        let p = MockProvider::new(vec![
            LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "first".to_string(),
                }],
                stop_reason: "end_turn".to_string(),
            },
            LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "second".to_string(),
                }],
                stop_reason: "end_turn".to_string(),
            },
        ]);
        let r1 = p.complete(&[], &[]).await.expect("first");
        let r2 = p.complete(&[], &[]).await.expect("second");
        match (&r1.content[0], &r2.content[0]) {
            (ContentBlock::Text { text: t1 }, ContentBlock::Text { text: t2 }) => {
                assert_eq!(t1, "first");
                assert_eq!(t2, "second");
            }
            _ => panic!("expected Text blocks"),
        }
    }

    #[tokio::test]
    async fn mock_provider_exhausted_queue_returns_error() {
        let p = MockProvider::text("only one", "end_turn");
        p.complete(&[], &[]).await.expect("first call ok");
        let err = p.complete(&[], &[]).await.unwrap_err();
        assert!(err.to_string().contains("exhausted"));
    }

    #[tokio::test]
    async fn mock_provider_is_arc_dyn_llm_provider() {
        let p: Arc<dyn LlmProvider> = Arc::new(MockProvider::text("works", "end_turn"));
        let resp = p.complete(&[], &[]).await.expect("response");
        assert_eq!(resp.stop_reason, "end_turn");
    }

    #[tokio::test]
    async fn mock_provider_ignores_messages_and_tools() {
        let messages = vec![Message::user("ignored")];
        let tools = vec![serde_json::json!({"name": "ignored_tool"})];
        let p = MockProvider::text("still works", "end_turn");
        let resp = p.complete(&messages, &tools).await.expect("response");
        assert!(matches!(resp.content[0], ContentBlock::Text { .. }));
    }
}
