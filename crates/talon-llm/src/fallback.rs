//! Ordered fallback across multiple providers.
//!
//! Tries each provider in turn and returns the first success. Any error —
//! auth, rate limit, network, or bad response — advances to the next provider,
//! since the whole point of a fallback chain is to survive one provider being
//! down or misconfigured. If every provider fails, the last error is returned.

use std::{future::Future, pin::Pin, sync::Arc};

use crate::{LlmError, LlmProvider, LlmResponse, Message};

pub struct FallbackProvider {
    providers: Vec<Arc<dyn LlmProvider>>,
}

impl FallbackProvider {
    pub fn new(providers: Vec<Arc<dyn LlmProvider>>) -> Self {
        Self { providers }
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

impl LlmProvider for FallbackProvider {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        tools: &'a [serde_json::Value],
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, LlmError>> + Send + 'a>> {
        Box::pin(async move {
            let mut last_err: Option<LlmError> = None;
            for (idx, provider) in self.providers.iter().enumerate() {
                match provider.complete(messages, tools).await {
                    Ok(resp) => return Ok(resp),
                    Err(e) => {
                        tracing::warn!("fallback provider {idx} failed: {e}; trying next");
                        last_err = Some(e);
                    }
                }
            }
            Err(last_err.unwrap_or_else(|| {
                LlmError::InvalidResponse("no providers configured".to_string())
            }))
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::MockProvider;

    /// A provider whose queue is empty always errors — handy for forcing fallback.
    fn always_fails() -> Arc<dyn LlmProvider> {
        Arc::new(MockProvider::new(vec![]))
    }

    #[tokio::test]
    async fn first_success_is_returned() {
        let chain = FallbackProvider::new(vec![
            Arc::new(MockProvider::text("primary", "end_turn")),
            always_fails(),
        ]);
        let resp = chain.complete(&[], &[]).await.expect("ok");
        assert_eq!(resp.stop_reason, "end_turn");
    }

    #[tokio::test]
    async fn falls_back_to_next_on_error() {
        let chain = FallbackProvider::new(vec![
            always_fails(),
            Arc::new(MockProvider::text("backup", "end_turn")),
        ]);
        let resp = chain.complete(&[], &[]).await.expect("backup used");
        match &resp.content[0] {
            crate::ContentBlock::Text { text } => assert_eq!(text, "backup"),
            _ => panic!("expected text"),
        }
    }

    #[tokio::test]
    async fn all_failing_returns_last_error() {
        let chain = FallbackProvider::new(vec![always_fails(), always_fails()]);
        assert!(chain.complete(&[], &[]).await.is_err());
    }

    #[tokio::test]
    async fn empty_chain_errors() {
        let chain = FallbackProvider::new(vec![]);
        assert!(chain.is_empty());
        let err = chain.complete(&[], &[]).await.unwrap_err();
        assert!(err.to_string().contains("no providers"));
    }

    #[tokio::test]
    async fn is_arc_dyn_llm_provider() {
        let chain: Arc<dyn LlmProvider> = Arc::new(FallbackProvider::new(vec![Arc::new(
            MockProvider::text("x", "end_turn"),
        )]));
        assert!(chain.complete(&[], &[]).await.is_ok());
    }
}
