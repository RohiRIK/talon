mod anthropic;
pub use anthropic::AnthropicProvider;

mod openai;
pub use openai::OpenAIProvider;

#[cfg(any(test, feature = "github-copilot-provider"))]
pub(crate) mod openai_compat;

#[cfg(feature = "github-copilot-provider")]
mod github_copilot;
#[cfg(feature = "github-copilot-provider")]
pub use github_copilot::GitHubCopilotProvider;

#[cfg(any(test, feature = "mock"))]
pub mod mock;
#[cfg(any(test, feature = "mock"))]
pub use mock::MockProvider;

use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Type #4 — LLM provider abstraction.
///
/// Returns `Pin<Box<dyn Future>>` instead of `async fn` for dyn-compatibility.
/// `Arc<dyn LlmProvider>` is the load-bearing reference type. See ADR 0007.
pub trait LlmProvider: Send + Sync {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        tools: &'a [serde_json::Value],
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, LlmError>> + Send + 'a>>;
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub role: String,
    pub content: serde_json::Value,
}

impl Message {
    pub fn user(content: impl Into<serde_json::Value>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<serde_json::Value>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
        }
    }
}

/// Content block within an LLM response — either plain text or a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug)]
pub struct LlmResponse {
    pub content: Vec<ContentBlock>,
    pub stop_reason: String,
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("authentication failed")]
    AuthFailed,
    #[error("rate limited — back off and retry")]
    RateLimited,
    #[error("invalid response: {0}")]
    InvalidResponse(String),
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn message_user_sets_role() {
        let m = Message::user("hello");
        assert_eq!(m.role, "user");
    }

    #[test]
    fn message_assistant_sets_role() {
        let m = Message::assistant(json!([]));
        assert_eq!(m.role, "assistant");
    }

    #[test]
    fn content_block_text_serializes_correctly() {
        let block = ContentBlock::Text {
            text: "hello".to_string(),
        };
        let val = serde_json::to_value(&block).expect("serialize");
        assert_eq!(val["type"], "text");
        assert_eq!(val["text"], "hello");
    }

    #[test]
    fn content_block_tool_use_serializes_with_snake_case_type() {
        let block = ContentBlock::ToolUse {
            id: "toolu_01".to_string(),
            name: "read_file".to_string(),
            input: json!({ "path": "./Cargo.toml" }),
        };
        let val = serde_json::to_value(&block).expect("serialize");
        assert_eq!(val["type"], "tool_use", "Anthropic API requires snake_case type tag");
        assert_eq!(val["id"], "toolu_01");
        assert_eq!(val["name"], "read_file");
        assert_eq!(val["input"]["path"], "./Cargo.toml");
    }

    #[test]
    fn content_block_text_deserializes() {
        let raw = r#"{"type":"text","text":"edition 2024"}"#;
        let block: ContentBlock = serde_json::from_str(raw).expect("deserialize");
        assert!(matches!(block, ContentBlock::Text { ref text } if text == "edition 2024"));
    }

    #[test]
    fn content_block_tool_use_deserializes() {
        let raw = r#"{"type":"tool_use","id":"t1","name":"echo","input":{"message":"hi"}}"#;
        let block: ContentBlock = serde_json::from_str(raw).expect("deserialize");
        match block {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "t1");
                assert_eq!(name, "echo");
                assert_eq!(input["message"], "hi");
            }
            ContentBlock::Text { .. } => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn content_block_round_trips() {
        let original = ContentBlock::ToolUse {
            id: "t2".to_string(),
            name: "read_file".to_string(),
            input: json!({ "path": "/tmp/x" }),
        };
        let serialized = serde_json::to_string(&original).expect("serialize");
        let deserialized: ContentBlock = serde_json::from_str(&serialized).expect("deserialize");
        let re_serialized = serde_json::to_string(&deserialized).expect("re-serialize");
        assert_eq!(serialized, re_serialized);
    }

    #[test]
    fn llm_error_auth_failed_display() {
        let err = LlmError::AuthFailed;
        assert!(err.to_string().contains("authentication"));
    }

    #[test]
    fn llm_error_rate_limited_display() {
        let err = LlmError::RateLimited;
        assert!(err.to_string().contains("rate"));
    }

    #[test]
    fn llm_error_invalid_response_display() {
        let err = LlmError::InvalidResponse("bad json".to_string());
        assert!(err.to_string().contains("bad json"));
    }
}
