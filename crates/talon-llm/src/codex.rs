use std::{future::Future, pin::Pin};

use reqwest::Client;

use crate::{LlmError, LlmProvider, LlmResponse, Message, openai_compat};

/// Auth: `OPENAI_API_KEY` env → `CODEX_ACCESS_TOKEN` env. Default model: `o4-mini`.
pub struct CodexProvider {
    client: Client,
    token: String,
    model: String,
}

impl CodexProvider {
    const ENDPOINT: &'static str = "https://api.openai.com/v1/chat/completions";
    const DEFAULT_MODEL: &'static str = "o4-mini";

    pub fn new() -> Result<Self, LlmError> {
        let token = Self::resolve_token()?;
        let model =
            std::env::var("TALON_LLM_MODEL").unwrap_or_else(|_| Self::DEFAULT_MODEL.to_string());
        Ok(Self {
            client: Client::new(),
            token,
            model,
        })
    }

    fn resolve_token() -> Result<String, LlmError> {
        if let Ok(tok) = std::env::var("OPENAI_API_KEY")
            && !tok.trim().is_empty()
        {
            return Ok(tok.trim().to_string());
        }
        if let Ok(tok) = std::env::var("CODEX_ACCESS_TOKEN")
            && !tok.trim().is_empty()
        {
            return Ok(tok.trim().to_string());
        }
        Err(LlmError::AuthFailed)
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

impl LlmProvider for CodexProvider {
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
    fn resolve_token_reads_openai_api_key() {
        // SAFETY: nextest runs each test in its own process.
        unsafe {
            std::env::set_var("OPENAI_API_KEY", "sk-test_abc");
            std::env::remove_var("CODEX_ACCESS_TOKEN");
        }
        let token = CodexProvider::resolve_token().expect("token");
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
        assert_eq!(token, "sk-test_abc");
    }

    #[test]
    fn resolve_token_falls_back_to_codex_access_token() {
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
            std::env::set_var("CODEX_ACCESS_TOKEN", "codex_token_xyz");
        }
        let token = CodexProvider::resolve_token().expect("token");
        unsafe {
            std::env::remove_var("CODEX_ACCESS_TOKEN");
        }
        assert_eq!(token, "codex_token_xyz");
    }

    #[test]
    fn resolve_token_trims_whitespace() {
        unsafe {
            std::env::set_var("OPENAI_API_KEY", "  sk-trimmed  ");
            std::env::remove_var("CODEX_ACCESS_TOKEN");
        }
        let token = CodexProvider::resolve_token().expect("token");
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
        assert_eq!(token, "sk-trimmed");
    }

    #[test]
    fn empty_openai_key_falls_back_to_codex_token() {
        unsafe {
            std::env::set_var("OPENAI_API_KEY", "   ");
            std::env::set_var("CODEX_ACCESS_TOKEN", "codex_fallback");
        }
        let token = CodexProvider::resolve_token().expect("token");
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("CODEX_ACCESS_TOKEN");
        }
        assert_eq!(token, "codex_fallback");
    }

    #[test]
    fn auth_fails_when_both_env_vars_absent() {
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("CODEX_ACCESS_TOKEN");
        }
        let result = CodexProvider::resolve_token();
        assert!(matches!(result, Err(LlmError::AuthFailed)));
    }

    #[test]
    fn default_model_is_o4_mini() {
        unsafe {
            std::env::set_var("OPENAI_API_KEY", "sk-dummy");
            std::env::remove_var("TALON_LLM_MODEL");
        }
        let p = CodexProvider::new().expect("new");
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
        assert_eq!(p.model, "o4-mini");
    }

    #[test]
    fn model_override_via_env() {
        unsafe {
            std::env::set_var("OPENAI_API_KEY", "sk-dummy");
            std::env::set_var("TALON_LLM_MODEL", "gpt-4o");
        }
        let p = CodexProvider::new().expect("new");
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("TALON_LLM_MODEL");
        }
        assert_eq!(p.model, "gpt-4o");
    }

    #[test]
    fn arc_dyn_llm_provider_is_constructible() {
        unsafe {
            std::env::set_var("OPENAI_API_KEY", "sk-dummy");
        }
        let p: Arc<dyn LlmProvider> = Arc::new(CodexProvider::new().expect("new"));
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
        let _ = p;
    }
}
