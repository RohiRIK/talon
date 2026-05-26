# Ollama Local Provider

> **Status:** ✅ Complete
> **Category:** API Bindings

---

## 1. Overview

Ollama exposes an OpenAI-compatible `/v1/chat/completions` endpoint,
so `OpenAiCompatClient` works out of the box with minimal config.
However, Ollama has unique operational concerns that warrant its own
wrapper: model management, pull-on-demand, health checking, and
GPU memory awareness.

---

## 2. Configuration

```toml
[llm.providers.ollama]
type = "ollama"
base_url = "http://localhost:11434"
model = "qwen2.5-coder:32b"
pull_if_missing = true          # auto-pull model on first use
gpu_layers = -1                 # -1 = all layers on GPU
context_length = 32768

[llm.providers.ollama.fallback]
# If Ollama is unreachable, fall through to this provider
provider = "openrouter"
```

---

## 3. OllamaClient

```rust
pub struct OllamaClient {
    inner: OpenAiCompatClient,   // delegates all completions here
    management_client: reqwest::Client,
    base_url: String,
    model: String,
    pull_if_missing: bool,
}

impl OllamaClient {
    pub async fn new(config: &OllamaConfig) -> Result<Self, LlmError> {
        // Health check
        let mgmt = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()?;

        let health = mgmt.get(format!("{}/api/tags", config.base_url))
            .send()
            .await;

        match health {
            Err(e) => {
                tracing::warn!("Ollama not reachable at {}: {e}", config.base_url);
                if config.required.unwrap_or(false) {
                    return Err(LlmError::ProviderUnavailable("ollama".into()));
                }
            }
            Ok(resp) if !resp.status().is_success() => {
                return Err(LlmError::ProviderUnavailable("ollama".into()));
            }
            Ok(_) => {
                tracing::info!("Ollama reachable at {}", config.base_url);
            }
        }

        let inner = OpenAiCompatClient::new(&OpenAiProviderConfig {
            base_url: Some(format!("{}/v1", config.base_url)),
            api_key: "ollama".into(),
            model: config.model.clone(),
            ..Default::default()
        })?;

        Ok(Self {
            inner,
            management_client: mgmt,
            base_url: config.base_url.clone(),
            model: config.model.clone(),
            pull_if_missing: config.pull_if_missing.unwrap_or(false),
        })
    }
}
```

---

## 4. Model Management

```rust
impl OllamaClient {
    /// Check if model is available locally
    pub async fn model_exists(&self) -> Result<bool, LlmError> {
        let resp: serde_json::Value = self.management_client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await?
            .json()
            .await?;

        let exists = resp["models"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|m| m["name"].as_str() == Some(&self.model));

        Ok(exists)
    }

    /// Pull model with progress streaming
    pub async fn pull_model(&self) -> Result<(), LlmError> {
        tracing::info!(model = self.model, "Pulling Ollama model...");

        let resp = self.management_client
            .post(format!("{}/api/pull", self.base_url))
            .json(&serde_json::json!({ "name": self.model, "stream": true }))
            .send()
            .await?;

        let mut stream = resp.bytes_stream();
        let mut last_status = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if let Ok(line) = std::str::from_utf8(&chunk) {
                for l in line.lines() {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(l) {
                        let status = v["status"].as_str().unwrap_or("").to_string();
                        if status != last_status {
                            tracing::info!(model = self.model, status = status, "Pull progress");
                            last_status = status;
                        }
                        if v["status"].as_str() == Some("success") {
                            return Ok(());
                        }
                        if let Some(err) = v["error"].as_str() {
                            return Err(LlmError::OllamaPullFailed(err.to_string()));
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// List available local models
    pub async fn list_models(&self) -> Result<Vec<OllamaModel>, LlmError> {
        let resp: serde_json::Value = self.management_client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await?
            .json()
            .await?;

        let models = resp["models"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|m| serde_json::from_value(m.clone()).ok())
            .collect();

        Ok(models)
    }
}

#[derive(Debug, Deserialize)]
pub struct OllamaModel {
    pub name: String,
    pub size: u64,
    pub modified_at: String,
    pub details: OllamaModelDetails,
}

#[derive(Debug, Deserialize)]
pub struct OllamaModelDetails {
    pub parameter_size: String,
    pub quantization_level: String,
    pub family: String,
}
```

---

## 5. LlmProvider Implementation

```rust
#[async_trait]
impl LlmProvider for OllamaClient {
    fn name(&self) -> &str { "ollama" }
    fn model(&self) -> &str { &self.model }

    async fn complete(
        &self,
        req: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<Delta, LlmError>>, LlmError> {
        // Auto-pull if model missing
        if self.pull_if_missing && !self.model_exists().await? {
            self.pull_model().await?;
        }

        // Delegate to OpenAI-compat inner client
        self.inner.complete(req).await
    }

    async fn count_tokens(&self, messages: &[Message]) -> Result<u32, LlmError> {
        // Ollama doesn't have a token counting endpoint
        // Use tiktoken estimate or character-based heuristic
        Ok(messages.iter()
            .map(|m| m.content.text_len() / 4)  // ~4 chars per token
            .sum::<usize>() as u32)
    }
}
```

---

## 6. FallbackProvider

When Ollama is offline, fall through to a cloud provider:

```rust
pub struct FallbackProvider {
    primary: Arc<dyn LlmProvider>,
    fallback: Arc<dyn LlmProvider>,
}

#[async_trait]
impl LlmProvider for FallbackProvider {
    async fn complete(
        &self,
        req: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<Delta, LlmError>>, LlmError> {
        match self.primary.complete(req.clone()).await {
            Ok(stream) => Ok(stream),
            Err(e) if e.is_connectivity_error() => {
                tracing::warn!(
                    primary = self.primary.name(),
                    fallback = self.fallback.name(),
                    error = %e,
                    "Primary provider failed — falling back"
                );
                self.fallback.complete(req).await
            }
            Err(e) => Err(e),
        }
    }
}
```

---

## 7. Recommended Local Models (2025–2026)

| Use Case | Model | Size | Notes |
|----------|-------|------|-------|
| General agent | `qwen2.5:32b` | 20GB | Best open-source for tool use |
| Code tasks | `qwen2.5-coder:32b` | 20GB | Superior code generation |
| Fast/cheap | `qwen2.5:7b` | 5GB | Good enough for simple tasks |
| Embedding | `nomic-embed-text` | 274MB | FTS5 complement for [semantic search](../07_Memory_System/59_Embedding_Retrieval.md) |
| Vision | `llava:34b` | 20GB | For browser_vision fallback |
---

## Related Documents

### Depends On
- [LLM Provider Abstraction](41_LLM_Provider_Abstraction.md)

### See Also
- [Config System](../02_Architecture/18a_Config_System.md)
- [OpenAI Provider](42_OpenAI_Provider.md)

