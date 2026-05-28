use std::{future::Future, pin::Pin};

use reqwest::Client;

use crate::{LlmError, LlmProvider, LlmResponse, Message, openai_compat};

/// Auth: `GEMINI_API_KEY` env → `GOOGLE_API_KEY` env → `agy auth token` CLI.
/// Default model: `gemini-3.5-flash`.
/// Endpoint: Google's official OpenAI-compatible Gemini endpoint.
pub struct AntigravityProvider {
    client: Client,
    token: String,
    model: String,
}

impl AntigravityProvider {
    /// Google's official OpenAI-compatible endpoint; accepts `GEMINI_API_KEY` as Bearer token.
    const ENDPOINT: &'static str =
        "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions";
    const DEFAULT_MODEL: &'static str = "gemini-3.5-flash";

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
        if let Ok(tok) = std::env::var("GEMINI_API_KEY")
            && !tok.trim().is_empty()
        {
            return Ok(tok.trim().to_string());
        }
        if let Ok(tok) = std::env::var("GOOGLE_API_KEY")
            && !tok.trim().is_empty()
        {
            return Ok(tok.trim().to_string());
        }
        // Blocking subprocess is acceptable here — called once at construction.
        // Fails gracefully if `agy` is not installed.
        let output = std::process::Command::new("agy")
            .args(["auth", "token"])
            .output()
            .map_err(|_| LlmError::AuthFailed)?;
        if !output.status.success() {
            return Err(LlmError::AuthFailed);
        }
        let tok = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if tok.is_empty() {
            return Err(LlmError::AuthFailed);
        }
        Ok(tok)
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

impl LlmProvider for AntigravityProvider {
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

    fn clear_auth_env() {
        // SAFETY: nextest runs each test in its own process.
        unsafe {
            std::env::remove_var("GEMINI_API_KEY");
            std::env::remove_var("GOOGLE_API_KEY");
        }
    }

    #[test]
    fn resolve_token_reads_gemini_api_key() {
        clear_auth_env();
        unsafe {
            std::env::set_var("GEMINI_API_KEY", "AIzaSy_test");
        }
        let token = AntigravityProvider::resolve_token().expect("token");
        clear_auth_env();
        assert_eq!(token, "AIzaSy_test");
    }

    #[test]
    fn resolve_token_falls_back_to_google_api_key() {
        clear_auth_env();
        unsafe {
            std::env::set_var("GOOGLE_API_KEY", "AIzaSy_google");
        }
        let token = AntigravityProvider::resolve_token().expect("token");
        clear_auth_env();
        assert_eq!(token, "AIzaSy_google");
    }

    #[test]
    fn gemini_key_takes_priority_over_google_key() {
        clear_auth_env();
        unsafe {
            std::env::set_var("GEMINI_API_KEY", "gemini_first");
            std::env::set_var("GOOGLE_API_KEY", "google_second");
        }
        let token = AntigravityProvider::resolve_token().expect("token");
        clear_auth_env();
        assert_eq!(token, "gemini_first");
    }

    #[test]
    fn resolve_token_trims_whitespace() {
        clear_auth_env();
        unsafe {
            std::env::set_var("GEMINI_API_KEY", "  AIzaSy_trimmed  ");
        }
        let token = AntigravityProvider::resolve_token().expect("token");
        clear_auth_env();
        assert_eq!(token, "AIzaSy_trimmed");
    }

    #[test]
    fn empty_gemini_key_falls_back_to_google_key() {
        clear_auth_env();
        unsafe {
            std::env::set_var("GEMINI_API_KEY", "   ");
            std::env::set_var("GOOGLE_API_KEY", "AIzaSy_fallback");
        }
        let token = AntigravityProvider::resolve_token().expect("token");
        clear_auth_env();
        assert_eq!(token, "AIzaSy_fallback");
    }

    #[test]
    fn default_model_is_gemini_flash() {
        clear_auth_env();
        unsafe {
            std::env::set_var("GEMINI_API_KEY", "AIzaSy_dummy");
            std::env::remove_var("TALON_LLM_MODEL");
        }
        let p = AntigravityProvider::new().expect("new");
        clear_auth_env();
        assert_eq!(p.model, "gemini-3.5-flash");
    }

    #[test]
    fn model_override_via_env() {
        clear_auth_env();
        unsafe {
            std::env::set_var("GEMINI_API_KEY", "AIzaSy_dummy");
            std::env::set_var("TALON_LLM_MODEL", "gemini-3-pro");
        }
        let p = AntigravityProvider::new().expect("new");
        unsafe {
            std::env::remove_var("TALON_LLM_MODEL");
        }
        clear_auth_env();
        assert_eq!(p.model, "gemini-3-pro");
    }

    #[test]
    fn arc_dyn_llm_provider_is_constructible() {
        clear_auth_env();
        unsafe {
            std::env::set_var("GEMINI_API_KEY", "AIzaSy_dummy");
        }
        let p: Arc<dyn LlmProvider> = Arc::new(AntigravityProvider::new().expect("new"));
        clear_auth_env();
        let _ = p;
    }
}
