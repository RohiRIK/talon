use std::{future::Future, pin::Pin};

use reqwest::Client;

use crate::{openai_compat, LlmError, LlmProvider, LlmResponse, Message};

/// Auth: `GITHUB_TOKEN` env → `gh auth token` CLI. Default model: `claude-sonnet-4-5`.
pub struct GitHubCopilotProvider {
    client: Client,
    token: String,
    model: String,
}

impl GitHubCopilotProvider {
    const ENDPOINT: &'static str = "https://api.githubcopilot.com/chat/completions";
    const DEFAULT_MODEL: &'static str = "claude-sonnet-4.6";

    pub fn new() -> Result<Self, LlmError> {
        let token = Self::resolve_token()?;
        let model = std::env::var("TALON_LLM_MODEL")
            .unwrap_or_else(|_| Self::DEFAULT_MODEL.to_string());
        Ok(Self { client: Client::new(), token, model })
    }

    fn resolve_token() -> Result<String, LlmError> {
        if let Ok(tok) = std::env::var("GITHUB_TOKEN")
            && !tok.trim().is_empty()
        {
            return Ok(tok.trim().to_string());
        }
        // Blocking subprocess is acceptable here — called once at construction.
        let output = std::process::Command::new("gh")
            .args(["auth", "token"])
            .output()
            .map_err(|_| LlmError::AuthFailed)?;
        if !output.status.success() {
            return Err(LlmError::AuthFailed);
        }
        let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if token.is_empty() {
            return Err(LlmError::AuthFailed);
        }
        Ok(token)
    }

    async fn complete_inner(
        &self,
        messages: &[Message],
        tools: &[serde_json::Value],
    ) -> Result<LlmResponse, LlmError> {
        let body = openai_compat::build_body(&self.model, messages, tools);

        let resp = self
            .client
            .post(Self::ENDPOINT)
            .bearer_auth(&self.token)
            .header("content-type", "application/json")
            .header("editor-version", "talon/0.2.0")
            .header("editor-plugin-version", "talon-llm/0.2.0")
            .json(&body)
            .send()
            .await?;

        let resp = openai_compat::check_status(resp).await?;
        let raw: openai_compat::RawResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;
        openai_compat::parse_response(raw)
    }
}

impl LlmProvider for GitHubCopilotProvider {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        tools: &'a [serde_json::Value],
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, LlmError>> + Send + 'a>> {
        Box::pin(self.complete_inner(messages, tools))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn resolve_token_reads_github_token_env() {
        // SAFETY: nextest runs each test in its own process.
        unsafe {
            std::env::set_var("GITHUB_TOKEN", "gho_test_token_abc");
        }
        let token = GitHubCopilotProvider::resolve_token().expect("token");
        unsafe {
            std::env::remove_var("GITHUB_TOKEN");
        }
        assert_eq!(token, "gho_test_token_abc");
    }

    #[test]
    fn resolve_token_trims_whitespace() {
        unsafe {
            std::env::set_var("GITHUB_TOKEN", "  gho_trimmed  ");
        }
        let token = GitHubCopilotProvider::resolve_token().expect("token");
        unsafe {
            std::env::remove_var("GITHUB_TOKEN");
        }
        assert_eq!(token, "gho_trimmed");
    }

    #[test]
    fn default_model_is_claude_sonnet() {
        unsafe {
            std::env::set_var("GITHUB_TOKEN", "gho_dummy");
            std::env::remove_var("TALON_LLM_MODEL");
        }
        let provider = GitHubCopilotProvider::new().expect("new");
        unsafe {
            std::env::remove_var("GITHUB_TOKEN");
        }
        assert_eq!(provider.model, "claude-sonnet-4.6");
    }

    #[test]
    fn model_override_via_env() {
        unsafe {
            std::env::set_var("GITHUB_TOKEN", "gho_dummy");
            std::env::set_var("TALON_LLM_MODEL", "gpt-4o");
        }
        let provider = GitHubCopilotProvider::new().expect("new");
        unsafe {
            std::env::remove_var("GITHUB_TOKEN");
            std::env::remove_var("TALON_LLM_MODEL");
        }
        assert_eq!(provider.model, "gpt-4o");
    }

    #[test]
    fn arc_dyn_llm_provider_is_constructible() {
        unsafe {
            std::env::set_var("GITHUB_TOKEN", "gho_dummy");
        }
        let provider: Arc<dyn LlmProvider> =
            Arc::new(GitHubCopilotProvider::new().expect("new"));
        unsafe {
            std::env::remove_var("GITHUB_TOKEN");
        }
        let _ = provider;
    }

    #[test]
    fn empty_github_token_env_is_rejected() {
        unsafe {
            std::env::set_var("GITHUB_TOKEN", "   ");
        }
        let result = GitHubCopilotProvider::resolve_token();
        unsafe {
            std::env::remove_var("GITHUB_TOKEN");
        }
        // gh CLI may or may not be available; either way the token must not be empty.
        if let Ok(tok) = result {
            assert!(!tok.is_empty(), "token must not be empty");
        }
    }

    /// Live smoke test — skipped by default.
    /// Run with: cargo nextest run --run-ignored all -E 'test(smoke)'
    #[tokio::test]
    #[ignore = "requires live GitHub token with Copilot access"]
    async fn smoke_complete_returns_text() {
        use crate::ContentBlock;
        use std::time::Duration;

        let provider = match GitHubCopilotProvider::new() {
            Ok(p) => p,
            Err(_) => return, // no token available in this environment
        };
        let messages = vec![Message::user("Say hello in one word.")];
        let resp = tokio::time::timeout(
            Duration::from_secs(30),
            provider.complete(&messages, &[]),
        )
        .await
        .expect("timed out after 30s")
        .expect("complete failed");

        assert!(!resp.content.is_empty(), "response must have at least one content block");
        let has_text = resp
            .content
            .iter()
            .any(|b| matches!(b, ContentBlock::Text { text } if !text.is_empty()));
        assert!(has_text, "response must contain a non-empty Text block");
    }
}
