# OpenAI-Compatible Provider

> **Status:** ✅ Complete
> **Category:** API Bindings

---

## 1. Scope

This provider handles all OpenAI-compatible APIs:

- **OpenAI** — `api.openai.com`
- **OpenRouter** — `openrouter.ai/api/v1`
- **Ollama** — `localhost:11434/v1`
- **vLLM** — any `/v1/chat/completions` endpoint
- **LM Studio**, **Groq**, **Together**, **Fireworks**, etc.

A single `OpenAiCompatClient` handles all of them via configurable `base_url`.

---

## 2. Struct & Config

```rust
#[derive(Debug, Clone)]
pub struct OpenAiCompatClient {
    client: reqwest::Client,
    base_url: String,
    api_key: SecretString,
    model: String,
    default_params: CompletionDefaults,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompletionDefaults {
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
}

fn default_max_tokens() -> u32 { 8192 }

impl OpenAiCompatClient {
    pub fn new(config: &OpenAiProviderConfig) -> Result<Self, LlmError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .pool_max_idle_per_host(4)
            .build()?;

        Ok(Self {
            client,
            base_url: config.base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".into()),
            api_key: SecretString::new(config.api_key.clone()),
            model: config.model.clone(),
            default_params: config.defaults.clone().unwrap_or_default(),
        })
    }
}
```

---

## 3. LlmProvider Trait Implementation

```rust
#[async_trait]
impl LlmProvider for OpenAiCompatClient {
    fn name(&self) -> &str { "openai-compat" }
    fn model(&self) -> &str { &self.model }

    async fn complete(
        &self,
        req: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<Delta, LlmError>>, LlmError> {
        let body = self.build_request_body(&req);

        let response = self.client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key.expose_secret()))
            .header("Content-Type", "application/json")
            // OpenRouter requires this
            .header("HTTP-Referer", "https://talon-agent.local")
            .json(&body)
            .send()
            .await
            .map_err(LlmError::Http)?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(LlmError::ApiError { status: status.as_u16(), body });
        }

        let stream = response.bytes_stream();
        Ok(Box::pin(parse_sse_stream(stream)))
    }

    async fn count_tokens(&self, messages: &[Message]) -> Result<u32, LlmError> {
        // Use tiktoken-rs for offline token counting
        // Falls back to character estimate if model not in tiktoken
        Ok(tiktoken_estimate(messages, &self.model))
    }
}
```

---

## 4. Request Body Construction

```rust
impl OpenAiCompatClient {
    fn build_request_body(&self, req: &CompletionRequest) -> serde_json::Value {
        let messages: Vec<serde_json::Value> = req.messages.iter()
            .map(|m| self.message_to_json(m))
            .collect();

        let mut body = serde_json::json!({
            "model": req.model.as_deref().unwrap_or(&self.model),
            "messages": messages,
            "stream": true,
            "max_tokens": req.max_tokens.unwrap_or(self.default_params.max_tokens),
        });

        if let Some(t) = req.temperature.or(self.default_params.temperature) {
            body["temperature"] = serde_json::json!(t);
        }

        // Tool definitions
        if !req.tools.is_empty() {
            body["tools"] = serde_json::json!(
                req.tools.iter().map(|t| serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })).collect::<Vec<_>>()
            );
            body["tool_choice"] = serde_json::json!("auto");
        }

        body
    }

    fn message_to_json(&self, msg: &Message) -> serde_json::Value {
        match &msg.content {
            MessageContent::Text(text) => serde_json::json!({
                "role": msg.role.as_str(),
                "content": text,
            }),
            MessageContent::Blocks(blocks) => serde_json::json!({
                "role": msg.role.as_str(),
                "content": blocks.iter().map(|b| match b {
                    ContentBlock::Text { text } => serde_json::json!({
                        "type": "text",
                        "text": text
                    }),
                    ContentBlock::ToolResult { tool_use_id, content } => serde_json::json!({
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": content
                    }),
                }).collect::<Vec<_>>(),
            }),
        }
    }
}
```

---

## 5. Retry Logic

```rust
pub struct RetryingLlmProvider {
    inner: Arc<dyn LlmProvider>,
    max_attempts: u32,
    backoff: ExponentialBackoff,
}

#[async_trait]
impl LlmProvider for RetryingLlmProvider {
    async fn complete(
        &self,
        req: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<Delta, LlmError>>, LlmError> {
        let mut attempt = 0;
        let mut delay = Duration::from_millis(500);

        loop {
            attempt += 1;
            match self.inner.complete(req.clone()).await {
                Ok(stream) => return Ok(stream),
                Err(e) if e.is_retryable() && attempt < self.max_attempts => {
                    tracing::warn!(attempt, error = %e, delay_ms = delay.as_millis(), "LLM retry");
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(30));
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl LlmError {
    pub fn is_retryable(&self) -> bool {
        matches!(self,
            LlmError::ApiError { status, .. } if matches!(status, 429 | 500 | 502 | 503 | 504)
        ) || matches!(self, LlmError::Http(_))
    }
}
```

---

## 6. Provider Configuration in TOML

```toml
[llm.providers.openai]
type = "openai_compat"
base_url = "https://api.openai.com/v1"
api_key = "${OPENAI_API_KEY}"
model = "gpt-4o"

[llm.providers.openrouter]
type = "openai_compat"
base_url = "https://openrouter.ai/api/v1"
api_key = "${OPENROUTER_API_KEY}"
model = "anthropic/claude-sonnet-4-5"

[llm.providers.ollama]
type = "openai_compat"
base_url = "http://localhost:11434/v1"
api_key = "ollama"
model = "llama3.2"

[llm.providers.copilot]
type = "openai_compat"
base_url = "https://api.githubcopilot.com"
api_key = "${GITHUB_TOKEN}"
model = "claude-sonnet-4-5"
```

Default provider selected via:
```toml
[llm]
default_provider = "openrouter"
```
---

## Related Documents

### Depends On
- [LLM Provider Abstraction](41_LLM_Provider_Abstraction.md)

### See Also
- [OpenAI-Compatible Client](42a_OpenAI_Compatible_Client.md)
- [Anthropic Provider](43_Anthropic_Provider.md)
- [Streaming SSE Parser](44_Streaming_SSE_Parser.md)

