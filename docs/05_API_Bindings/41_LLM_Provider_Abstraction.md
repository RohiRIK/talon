# LLM Provider Abstraction Layer

> **Last corrected:** dogfood pass 2
>
> **Status:** ✅ Complete
> **Category:** API Bindings

---

## 1. Design Goals

1. **Single trait** — `AgentLoop` never references a concrete provider
2. **Streaming-first** — All responses are `Stream<Item=Delta>`, even if provider buffers
3. **Provider feature flags** — Vision, tool use, JSON mode surfaced as capability queries
4. **Hot-swap** — Switch provider per-session via config or `--model` flag
5. **OpenAI-compatible default** — 90% of providers expose the same REST API

---

## 2. Core Types

```rust
// talon-llm/src/message.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role { System, User, Assistant, Tool }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    Image { source: ImageSource },
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: Vec<ContentBlock>, is_error: bool },
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Option<Vec<serde_json::Value>>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub system: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Delta {
    Text(String),
    ToolCallStart { id: String, name: String },
    ToolCallDelta { id: String, json_fragment: String },
    ToolCallEnd { id: String },
    Usage { input_tokens: u32, output_tokens: u32 },
    Done,
}
```

---

## 3. The `LlmProvider` Trait

```rust
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

// Stream alias for object safety — Pin<Box<dyn Stream>> is object-safe,
// `impl Stream` return is NOT (it makes the trait non-object-safe).
pub type DeltaStream = Pin<Box<dyn Stream<Item = Result<Delta, LlmError>> + Send + 'static>>;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn id(&self) -> &str;
    fn default_model(&self) -> &str;
    fn available_models(&self) -> Vec<String> { vec![] }

    // Capability queries
    fn supports_vision(&self) -> bool { false }
    fn supports_tool_use(&self) -> bool { true }
    fn supports_json_mode(&self) -> bool { false }
    fn supports_streaming(&self) -> bool { true }
    fn context_window(&self, model: &str) -> u32 { 32_000 }

    /// Non-streaming completion — returns a single complete response.
    async fn complete(
        &self,
        req: CompletionRequest,
    ) -> Result<LlmResponse, LlmError>;

    /// Streaming completion — returns `Pin<Box<dyn Stream>>` so the trait
    /// remains object-safe and can be used as `Arc<dyn LlmProvider>`.
    async fn stream(
        &self,
        req: CompletionRequest,
    ) -> Result<DeltaStream, LlmError>;
}
```

---

## 4. OpenAI-Compatible Client

```rust
// Covers: OpenAI, OpenRouter, Ollama, LM Studio, vLLM, Together, Groq, Mistral
pub struct OpenAiCompatClient {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

#[async_trait]
impl LlmProvider for OpenAiCompatClient {
    fn id(&self) -> &str { "openai-compat" }
    fn default_model(&self) -> &str { &self.model }

    async fn complete(
        &self,
        req: CompletionRequest,
    ) -> Result<LlmResponse, LlmError> {
        // Non-streaming: set stream=false and parse a single JSON response
        let body = serde_json::json!({
            "model": req.model,
            "messages": req.messages,
            "tools": req.tools,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
            "stream": false,
        });
        let resp = self.client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send().await?
            .json::<LlmResponse>().await?;
        Ok(resp)
    }

    async fn stream(
        &self,
        req: CompletionRequest,
    ) -> Result<DeltaStream, LlmError> {
        let body = serde_json::json!({
            "model": req.model,
            "messages": req.messages,
            "tools": req.tools,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
            "stream": true,
        });

        let resp = self.client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send().await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let msg = resp.text().await.unwrap_or_default();
            return Err(LlmError::ApiError { status, message: msg });
        }

        Ok(Box::pin(sse_stream_to_deltas(resp.bytes_stream())))
    }
}

fn sse_stream_to_deltas(
    bytes: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = Result<Delta, LlmError>> + Send + 'static {
    use futures::StreamExt;
    use async_stream::stream;

    let mut tool_json_buf: HashMap<String, String> = HashMap::new();

    stream! {
        let mut lines = bytes
            .flat_map(|b| {
                let s = String::from_utf8_lossy(&b.unwrap_or_default()).to_string();
                futures::stream::iter(s.lines().map(|l| l.to_string()).collect::<Vec<_>>())
            });

        while let Some(line) = lines.next().await {
            if !line.starts_with("data: ") { continue; }
            let data = &line[6..];
            if data == "[DONE]" {
                yield Ok(Delta::Done);
                break;
            }
            if let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data) {
                for delta in parse_openai_chunk(&chunk, &mut tool_json_buf) {
                    yield Ok(delta);
                }
            }
        }
    }
}
```

---

## 5. Anthropic Direct Client

Anthropic's API differs from OpenAI: `system` is a top-level field, content blocks use different tags, streaming uses `event:` SSE type headers.

```rust
pub struct AnthropicClient {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

#[async_trait]
impl LlmProvider for AnthropicClient {
    fn id(&self) -> &str { "anthropic" }
    fn supports_vision(&self) -> bool { true }
    fn context_window(&self, model: &str) -> u32 {
        if model.contains("claude-3-5") { 200_000 } else { 100_000 }
    }

    async fn complete(&self, req: CompletionRequest) -> Result<LlmResponse, LlmError> {
        let body = anthropic_request_body(&req)?;
        let resp = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send().await?;
        Ok(parse_anthropic_response(resp).await?)
    }

    async fn stream(&self, req: CompletionRequest) -> Result<DeltaStream, LlmError> {
        let body = anthropic_request_body(&req)?;
        let resp = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send().await?;
        // parse Anthropic SSE format
        Ok(Box::pin(anthropic_sse_stream(resp.bytes_stream())))
    }
}
```

---

## 6. Provider Registry

```rust
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn LlmProvider>>,
    default: String,
}

impl ProviderRegistry {
    pub fn from_config(cfg: &LlmConfig) -> Result<Self, LlmError> {
        let mut reg = Self { providers: HashMap::new(), default: cfg.default_provider.clone() };

        for (id, provider_cfg) in &cfg.providers {
            let provider: Arc<dyn LlmProvider> = match provider_cfg.r#type.as_str() {
                "openai" | "openrouter" | "ollama" | "groq" => {
                    Arc::new(OpenAiCompatClient::from_config(id, provider_cfg)?)
                }
                "anthropic" => Arc::new(AnthropicClient::from_config(provider_cfg)?),
                unknown => return Err(LlmError::UnknownProvider(unknown.into())),
            };
            reg.providers.insert(id.clone(), provider);
        }

        Ok(reg)
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn LlmProvider>> {
        self.providers.get(id).cloned()
    }

    pub fn default_provider(&self) -> Arc<dyn LlmProvider> {
        self.providers[&self.default].clone()
    }
}
```

---

## 7. Config Schema

```toml
[llm]
default_provider = "anthropic"

[llm.providers.anthropic]
type = "anthropic"
api_key = "${ANTHROPIC_API_KEY}"
model = "claude-sonnet-4-5"

[llm.providers.openai]
type = "openai"
base_url = "https://api.openai.com/v1"
api_key = "${OPENAI_API_KEY}"
model = "gpt-4o"

[llm.providers.local]
type = "ollama"
base_url = "http://localhost:11434/v1"
api_key = "ollama"
model = "llama3.1:8b"

[llm.providers.openrouter]
type = "openrouter"
base_url = "https://openrouter.ai/api/v1"
api_key = "${OPENROUTER_API_KEY}"
model = "anthropic/claude-sonnet-4-5"
```
---

## Related Documents

### Depends On
- [Cargo Workspace Design](../02_Architecture/12_Workspace_And_Crate_Structure.md)
- [Error Handling Strategy](../06_Concurrency/54_Error_Handling_Strategy.md)

### Used By
- [Core Agent Loop Design](../02_Architecture/13_Core_Agent_Loop_Design.md)
- [OpenAI Provider](42_OpenAI_Provider.md)
- [Anthropic Provider](43_Anthropic_Provider.md)
- [Ollama Local Provider](45a_Ollama_Local_Provider.md)

### See Also
- [Streaming SSE Parser](44_Streaming_SSE_Parser.md)
- [Streaming & Realtime Output](../04_Core_Features/31a_Streaming_And_Realtime_Output.md)

