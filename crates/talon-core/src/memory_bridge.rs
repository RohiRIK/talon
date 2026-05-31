//! Bridges between the LTM crate's injection traits and the live runtime.
//!
//! `talon-memory` defines `FactCompleter` and `Embedder` as local traits so it
//! never depends on `talon-llm` or an embedding model. The agent loop supplies
//! these production implementations: fact extraction runs on the real
//! `LlmProvider`, and — until the `semantic-search` model is wired — embeddings
//! degrade to a zero vector so promotion still writes facts (recall then falls
//! back to FTS5/BM25 only).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use talon_llm::{ContentBlock, LlmProvider, Message};
use talon_memory::{Embedder, FactCompleter, MemoryError};

/// Embedding width of the LTM vector store (`vec0 float[384]`).
pub const EMBED_DIM: usize = 384;

/// Runs LTM fact extraction prompts against a live `LlmProvider`. Extraction is
/// a single user-prompt completion with no tools exposed.
pub struct LlmFactCompleter {
    provider: Arc<dyn LlmProvider>,
}

impl LlmFactCompleter {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }
}

impl FactCompleter for LlmFactCompleter {
    fn complete<'a>(
        &'a self,
        prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, MemoryError>> + Send + 'a>> {
        Box::pin(async move {
            let messages = [Message::user(prompt)];
            let response = self
                .provider
                .complete(&messages, &[])
                .await
                .map_err(|e| MemoryError::Llm(e.to_string()))?;
            let text = response
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    ContentBlock::ToolUse { .. } => None,
                })
                .collect::<Vec<_>>()
                .join("");
            Ok(text)
        })
    }
}

/// Placeholder embedder used when no semantic model is configured. Every fact
/// embeds to the zero vector, so vector KNN cannot distinguish memories and
/// dedup never merges — recall relies on FTS5 instead. Swapped for a
/// fastembed-backed embedder under the `semantic-search` feature.
pub struct ZeroEmbedder;

impl Embedder for ZeroEmbedder {
    fn embed<'a>(
        &'a self,
        _text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<f32>, MemoryError>> + Send + 'a>> {
        Box::pin(async move { Ok(vec![0.0f32; EMBED_DIM]) })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use talon_llm::MockProvider;

    #[tokio::test]
    async fn fact_completer_returns_provider_text() {
        let provider = Arc::new(MockProvider::text("[{\"content\":\"x\"}]", "end_turn"));
        let completer = LlmFactCompleter::new(provider);
        let out = completer.complete("extract facts").await.expect("complete");
        assert_eq!(out, "[{\"content\":\"x\"}]");
    }

    #[tokio::test]
    async fn zero_embedder_returns_zeroed_vector_of_embed_dim() {
        let v = ZeroEmbedder.embed("anything").await.expect("embed");
        assert_eq!(v.len(), EMBED_DIM);
        assert!(v.iter().all(|x| *x == 0.0));
    }
}
