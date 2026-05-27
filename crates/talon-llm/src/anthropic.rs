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
        let body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": messages,
            "tools": tools,
        });

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
    fn provider_stores_api_key() {
        let p = AnthropicProvider::new("sk-test-123".to_string());
        assert_eq!(p.api_key, "sk-test-123");
    }

    /// LlmProvider trait is dyn-compatible — Arc<dyn LlmProvider> must compile.
    /// This is the Type #4 dyn-compatibility check analogous to Arc<dyn Tool>.
    #[test]
    fn arc_dyn_llm_provider_is_constructible() {
        use std::sync::Arc;
        use crate::LlmProvider;
        let provider: Arc<dyn LlmProvider> = Arc::new(AnthropicProvider::new("key".to_string()));
        // If this compiles, the trait is dyn-compatible — the test body is the assertion.
        let _ = provider;
    }
}
