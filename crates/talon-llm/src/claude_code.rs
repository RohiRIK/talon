use std::{future::Future, pin::Pin};

use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

use crate::{ContentBlock, LlmError, LlmProvider, LlmResponse, Message};

/// Credential variant drives the auth header sent on each request.
enum AnthropicAuth {
    /// OAuth token or `ANTHROPIC_AUTH_TOKEN` — `Authorization: Bearer`.
    Bearer(String),
    /// `ANTHROPIC_API_KEY` — `x-api-key`.
    ApiKey(String),
}

/// Auth: `CLAUDE_CODE_OAUTH_TOKEN` (Bearer) → `ANTHROPIC_AUTH_TOKEN` (Bearer) →
/// `ANTHROPIC_API_KEY` (x-api-key) → `claude setup-token` CLI. Default model: `claude-opus-4-7`.
pub struct ClaudeCodeProvider {
    client: Client,
    auth: AnthropicAuth,
    model: String,
}

impl ClaudeCodeProvider {
    const ENDPOINT: &'static str = "https://api.anthropic.com/v1/messages";
    const DEFAULT_MODEL: &'static str = "claude-opus-4-7";

    pub fn new() -> Result<Self, LlmError> {
        let auth = Self::resolve_auth()?;
        let model =
            std::env::var("TALON_LLM_MODEL").unwrap_or_else(|_| Self::DEFAULT_MODEL.to_string());
        Ok(Self {
            client: Client::new(),
            auth,
            model,
        })
    }

    fn resolve_auth() -> Result<AnthropicAuth, LlmError> {
        // 1. CLAUDE_CODE_OAUTH_TOKEN → Bearer
        if let Ok(tok) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN")
            && !tok.trim().is_empty()
        {
            return Ok(AnthropicAuth::Bearer(tok.trim().to_string()));
        }
        // 2. ANTHROPIC_AUTH_TOKEN → Bearer
        if let Ok(tok) = std::env::var("ANTHROPIC_AUTH_TOKEN")
            && !tok.trim().is_empty()
        {
            return Ok(AnthropicAuth::Bearer(tok.trim().to_string()));
        }
        // 3. ANTHROPIC_API_KEY → x-api-key
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY")
            && !key.trim().is_empty()
        {
            return Ok(AnthropicAuth::ApiKey(key.trim().to_string()));
        }
        // 4. claude setup-token CLI — blocking subprocess, called once at construction.
        let output = std::process::Command::new("claude")
            .args(["setup-token"])
            .output()
            .map_err(|_| LlmError::AuthFailed)?;
        if !output.status.success() {
            return Err(LlmError::AuthFailed);
        }
        let tok = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if tok.is_empty() {
            return Err(LlmError::AuthFailed);
        }
        Ok(AnthropicAuth::Bearer(tok))
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

        let req = self
            .client
            .post(Self::ENDPOINT)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");

        let req = match &self.auth {
            AnthropicAuth::Bearer(tok) => req.bearer_auth(tok),
            AnthropicAuth::ApiKey(key) => req.header("x-api-key", key),
        };

        let resp = req.json(&body).send().await?;

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

impl LlmProvider for ClaudeCodeProvider {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        tools: &'a [serde_json::Value],
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, LlmError>> + Send + 'a>> {
        Box::pin(self.complete_inner(messages, tools))
    }
}

#[derive(Deserialize)]
struct RawResponse {
    content: Vec<ContentBlock>,
    stop_reason: String,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn clear_auth_env() {
        // SAFETY: nextest runs each test in its own process.
        unsafe {
            std::env::remove_var("CLAUDE_CODE_OAUTH_TOKEN");
            std::env::remove_var("ANTHROPIC_AUTH_TOKEN");
            std::env::remove_var("ANTHROPIC_API_KEY");
        }
    }

    #[test]
    fn oauth_token_is_first_priority() {
        clear_auth_env();
        unsafe {
            std::env::set_var("CLAUDE_CODE_OAUTH_TOKEN", "oauth_tok");
            std::env::set_var("ANTHROPIC_API_KEY", "sk-key");
        }
        let auth = ClaudeCodeProvider::resolve_auth().expect("auth");
        clear_auth_env();
        assert!(matches!(auth, AnthropicAuth::Bearer(t) if t == "oauth_tok"));
    }

    #[test]
    fn anthropic_auth_token_is_bearer() {
        clear_auth_env();
        unsafe {
            std::env::set_var("ANTHROPIC_AUTH_TOKEN", "bearer_tok");
        }
        let auth = ClaudeCodeProvider::resolve_auth().expect("auth");
        clear_auth_env();
        assert!(matches!(auth, AnthropicAuth::Bearer(t) if t == "bearer_tok"));
    }

    #[test]
    fn api_key_is_third_priority() {
        clear_auth_env();
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "sk-apikey");
        }
        let auth = ClaudeCodeProvider::resolve_auth().expect("auth");
        clear_auth_env();
        assert!(matches!(auth, AnthropicAuth::ApiKey(k) if k == "sk-apikey"));
    }

    #[test]
    fn auth_token_trims_whitespace() {
        clear_auth_env();
        unsafe {
            std::env::set_var("CLAUDE_CODE_OAUTH_TOKEN", "  tok_trimmed  ");
        }
        let auth = ClaudeCodeProvider::resolve_auth().expect("auth");
        clear_auth_env();
        assert!(matches!(auth, AnthropicAuth::Bearer(t) if t == "tok_trimmed"));
    }

    #[test]
    fn empty_oauth_token_falls_through_to_api_key() {
        clear_auth_env();
        unsafe {
            std::env::set_var("CLAUDE_CODE_OAUTH_TOKEN", "   ");
            std::env::set_var("ANTHROPIC_API_KEY", "sk-fallback");
        }
        let auth = ClaudeCodeProvider::resolve_auth().expect("auth");
        clear_auth_env();
        assert!(matches!(auth, AnthropicAuth::ApiKey(_)));
    }

    #[test]
    fn default_model_is_claude_opus() {
        clear_auth_env();
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "sk-dummy");
            std::env::remove_var("TALON_LLM_MODEL");
        }
        let p = ClaudeCodeProvider::new().expect("new");
        clear_auth_env();
        assert_eq!(p.model, "claude-opus-4-7");
    }

    #[test]
    fn model_override_via_env() {
        clear_auth_env();
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "sk-dummy");
            std::env::set_var("TALON_LLM_MODEL", "claude-sonnet-4-6");
        }
        let p = ClaudeCodeProvider::new().expect("new");
        unsafe {
            std::env::remove_var("TALON_LLM_MODEL");
        }
        clear_auth_env();
        assert_eq!(p.model, "claude-sonnet-4-6");
    }

    #[test]
    fn arc_dyn_llm_provider_is_constructible() {
        clear_auth_env();
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "sk-dummy");
        }
        let p: Arc<dyn LlmProvider> = Arc::new(ClaudeCodeProvider::new().expect("new"));
        clear_auth_env();
        let _ = p;
    }
}
