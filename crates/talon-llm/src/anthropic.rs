use std::{future::Future, pin::Pin};

use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

use crate::{ContentBlock, LlmError, LlmProvider, LlmResponse, Message};

/// Anthropic Messages API provider.
/// One `Client` per provider instance; `Client` manages the connection pool internally.
pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    model: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        let model = std::env::var("TALON_LLM_MODEL")
            .unwrap_or_else(|_| "claude-haiku-4-5-20251001".to_string());
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
        // Anthropic rejects role:"system" inside the messages array — it must be
        // hoisted to a top-level `system` field. Partition here so the agent loop
        // can stay provider-agnostic (it just prepends a Message::system).
        let (system, chat) = split_system(messages);

        let mut body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": chat,
            "tools": tools,
        });
        if let Some(system) = system {
            body["system"] = json!(system);
        }

        let resp = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
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

        Ok(LlmResponse {
            content: raw.content,
            stop_reason: raw.stop_reason,
        })
    }
}

impl LlmProvider for AnthropicProvider {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        tools: &'a [serde_json::Value],
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, LlmError>> + Send + 'a>> {
        Box::pin(self.complete_inner(messages, tools))
    }
}

/// Split system messages out of the conversation. Returns the concatenated
/// system text (Anthropic's top-level `system` field) and the remaining
/// non-system messages. Multiple system messages are joined with a blank line.
fn split_system(messages: &[Message]) -> (Option<String>, Vec<&Message>) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut chat: Vec<&Message> = Vec::new();
    for m in messages {
        if m.role == "system" {
            if let Some(text) = m.content.as_str() {
                system_parts.push(text.to_string());
            }
        } else {
            chat.push(m);
        }
    }
    let system = (!system_parts.is_empty()).then(|| system_parts.join("\n\n"));
    (system, chat)
}

/// Internal deserialization target — `LlmResponse` does not need to derive `Deserialize`.
#[derive(Deserialize)]
struct RawResponse {
    content: Vec<ContentBlock>,
    stop_reason: String,
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn provider_uses_default_model_when_env_absent() {
        // nextest isolates processes so this is safe without unsafe unset.
        let prior = std::env::var("TALON_LLM_MODEL").ok();
        if prior.is_some() {
            return; // skip — can't cleanly unset env in this process
        }
        let p = AnthropicProvider::new("key".to_string());
        assert_eq!(p.model, "claude-haiku-4-5-20251001");
    }

    #[test]
    fn provider_uses_env_model_override() {
        // SAFETY: nextest runs each test in an isolated process with no concurrent threads
        // accessing env vars, so set_var/remove_var are race-free here.
        unsafe {
            std::env::set_var("TALON_LLM_MODEL", "claude-opus-4-7");
        }
        let p = AnthropicProvider::new("key".to_string());
        // SAFETY: same process-isolation guarantee as above.
        unsafe {
            std::env::remove_var("TALON_LLM_MODEL");
        }
        assert_eq!(p.model, "claude-opus-4-7");
    }

    #[test]
    fn split_system_hoists_system_and_keeps_chat_order() {
        let msgs = vec![
            Message::system("memory is on sqlite"),
            Message::user("hello"),
            Message::assistant("hi"),
        ];
        let (system, chat) = split_system(&msgs);
        assert_eq!(system.as_deref(), Some("memory is on sqlite"));
        assert_eq!(chat.len(), 2);
        assert_eq!(chat[0].role, "user");
        assert_eq!(chat[1].role, "assistant");
    }

    #[test]
    fn split_system_joins_multiple_system_messages() {
        let msgs = vec![
            Message::system("first"),
            Message::system("second"),
            Message::user("hi"),
        ];
        let (system, chat) = split_system(&msgs);
        assert_eq!(system.as_deref(), Some("first\n\nsecond"));
        assert_eq!(chat.len(), 1);
    }

    #[test]
    fn split_system_is_none_without_system_messages() {
        let msgs = vec![Message::user("hi")];
        let (system, chat) = split_system(&msgs);
        assert!(system.is_none());
        assert_eq!(chat.len(), 1);
    }

    #[test]
    fn provider_stores_api_key() {
        let p = AnthropicProvider::new("sk-test-123".to_string());
        assert_eq!(p.api_key, "sk-test-123");
    }

    /// LlmProvider trait is dyn-compatible — Arc<dyn LlmProvider> must compile.
    /// This is the Type #4 dyn-compatibility check analogous to Arc<dyn Tool>.
    #[test]
    fn arc_dyn_llm_provider_is_constructible() {
        use crate::LlmProvider;
        use std::sync::Arc;
        let provider: Arc<dyn LlmProvider> = Arc::new(AnthropicProvider::new("key".to_string()));
        // If this compiles, the trait is dyn-compatible — the test body is the assertion.
        let _ = provider;
    }
}
