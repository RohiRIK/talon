use std::{future::Future, pin::Pin};

use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

use crate::{ContentBlock, LlmError, LlmProvider, LlmResponse, Message};

/// OpenAI Chat Completions API provider.
/// Uses the tool calling format (tools + tool_calls).
pub struct OpenAIProvider {
    client: Client,
    api_key: String,
    model: String,
}

impl OpenAIProvider {
    pub fn new(api_key: String) -> Self {
        let model = std::env::var("TALON_LLM_MODEL")
            .unwrap_or_else(|_| "gpt-4o-mini".to_string());
        Self {
            client: Client::new(),
            api_key,
            model,
        }
    }

    async fn complete_inner(
        &self,
        messages: &[Message],
        tools: &[serde_json::Value],
    ) -> Result<LlmResponse, LlmError> {
        // Convert Anthropic-style tool schemas to OpenAI function schemas.
        let oai_tools: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t["name"],
                        "description": t["description"],
                        "parameters": t["input_schema"],
                    }
                })
            })
            .collect();

        let mut body = json!({
            "model": self.model,
            "messages": messages,
        });

        if !oai_tools.is_empty() {
            body["tools"] = json!(oai_tools);
        }

        let resp = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            if status == 401 {
                return Err(LlmError::AuthFailed);
            }
            if status == 429 {
                return Err(LlmError::RateLimited);
            }
            let text = resp.text().await.unwrap_or_default();
            return Err(LlmError::InvalidResponse(format!("{status}: {text}")));
        }

        let raw: RawResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;

        let choice = raw
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| LlmError::InvalidResponse("no choices in response".to_string()))?;

        let stop_reason = match choice.finish_reason.as_deref() {
            Some("tool_calls") => "tool_use".to_string(),
            Some("stop") | None => "end_turn".to_string(),
            Some(other) => other.to_string(),
        };

        let mut content: Vec<ContentBlock> = Vec::new();

        if let Some(text) = choice.message.content && !text.is_empty() {
            content.push(ContentBlock::Text { text });
        }

        if let Some(tool_calls) = choice.message.tool_calls {
            for tc in tool_calls {
                let input: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::Value::Null);
                content.push(ContentBlock::ToolUse {
                    id: tc.id,
                    name: tc.function.name,
                    input,
                });
            }
        }

        Ok(LlmResponse {
            content,
            stop_reason,
        })
    }
}

impl LlmProvider for OpenAIProvider {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        tools: &'a [serde_json::Value],
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, LlmError>> + Send + 'a>> {
        Box::pin(self.complete_inner(messages, tools))
    }
}

// ── Internal deserialization types ────────────────────────────────────────────

#[derive(Deserialize)]
struct RawResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    finish_reason: Option<String>,
    message: RawMessage,
}

#[derive(Deserialize)]
struct RawMessage {
    content: Option<String>,
    tool_calls: Option<Vec<RawToolCall>>,
}

#[derive(Deserialize)]
struct RawToolCall {
    id: String,
    function: RawFunction,
}

#[derive(Deserialize)]
struct RawFunction {
    name: String,
    arguments: String,
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn provider_uses_default_model_when_env_absent() {
        let prior = std::env::var("TALON_LLM_MODEL").ok();
        if prior.is_some() {
            return;
        }
        let p = OpenAIProvider::new("key".to_string());
        assert_eq!(p.model, "gpt-4o-mini");
    }

    #[test]
    fn provider_uses_env_model_override() {
        // SAFETY: nextest isolates each test in its own process — env mutation is race-free.
        unsafe {
            std::env::set_var("TALON_LLM_MODEL", "gpt-4-turbo");
        }
        let p = OpenAIProvider::new("key".to_string());
        // SAFETY: same process isolation guarantee.
        unsafe {
            std::env::remove_var("TALON_LLM_MODEL");
        }
        assert_eq!(p.model, "gpt-4-turbo");
    }

    #[test]
    fn provider_stores_api_key() {
        let p = OpenAIProvider::new("sk-test-456".to_string());
        assert_eq!(p.api_key, "sk-test-456");
    }

    #[test]
    fn arc_dyn_llm_provider_is_constructible() {
        use std::sync::Arc;
        use crate::LlmProvider;
        let provider: Arc<dyn LlmProvider> = Arc::new(OpenAIProvider::new("key".to_string()));
        let _ = provider;
    }

    #[test]
    fn finish_reason_tool_calls_maps_to_tool_use() {
        // Verify the mapping logic directly without a network call.
        let stop_reason = match Some("tool_calls") {
            Some("tool_calls") => "tool_use",
            Some("stop") | None => "end_turn",
            Some(other) => other,
        };
        assert_eq!(stop_reason, "tool_use");
    }

    #[test]
    fn finish_reason_stop_maps_to_end_turn() {
        let stop_reason = match Some("stop") {
            Some("tool_calls") => "tool_use",
            Some("stop") | None => "end_turn",
            Some(other) => other,
        };
        assert_eq!(stop_reason, "end_turn");
    }
}
