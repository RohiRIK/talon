# OpenAI-Compatible Client

> **Status:** ✅ Complete
> **Category:** API Bindings

---

## 1. Overview

Many providers expose an OpenAI-compatible API (same JSON format,
different base URL). Talon implements one generic client that works for:
- OpenAI (api.openai.com)
- OpenRouter (openrouter.ai/api/v1)
- Groq (api.groq.com/openai/v1)
- Together AI, Anyscale, Fireworks, etc.
- Local: Ollama, LM Studio, vLLM (all OpenAI-compat)

---

## 2. Core Client

```rust
// talon-llm/src/providers/openai_compat.rs

pub struct OpenAiCompatClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    default_model: String,
}

impl OpenAiCompatClient {
    pub fn new(config: &OpenAiCompatConfig) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap(),
            base_url: config.base_url.trim_end_matches('/').to_string(),
            api_key: config.api_key.clone(),
            default_model: config.model.clone(),
        }
    }

    fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatClient {
    async fn complete(
        &self,
        request: LlmRequest,
    ) -> Result<LlmResponse, LlmError> {
        let body = self.build_request_body(&request, false);
        let resp: ChatCompletionResponse = self.http
            .post(self.chat_url())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send().await?
            .error_for_status()?
            .json().await?;

        Ok(LlmResponse {
            content: resp.choices[0].message.content.clone().unwrap_or_default(),
            tool_calls: self.parse_tool_calls(&resp.choices[0].message),
            stop_reason: resp.choices[0].finish_reason.parse().unwrap_or_default(),
            usage: resp.usage.map(|u| Usage {
                input_tokens: u.prompt_tokens,
                output_tokens: u.completion_tokens,
            }),
        })
    }

    async fn stream(
        &self,
        request: LlmRequest,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<LlmResponse, LlmError> {
        let body = self.build_request_body(&request, true);
        let resp = self.http
            .post(self.chat_url())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send().await?
            .error_for_status()?;

        let mut stream = resp.bytes_stream();
        let mut parser = OpenAiSseParser::new();
        let mut final_response = LlmResponse::default();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk?;
            for delta in parser.feed(&bytes) {
                match delta {
                    Delta::Text(t) => {
                        event_tx.send(AgentEvent::TextDelta { content: t.clone() }).await.ok();
                        final_response.content.push_str(&t);
                    }
                    Delta::ToolUseStart { id, name } => {
                        event_tx.send(AgentEvent::ToolCallStart { id, name }).await.ok();
                    }
                    Delta::ToolUseComplete { id, name, args } => {
                        let args: Value = serde_json::from_str(&args).unwrap_or(Value::Null);
                        final_response.tool_calls.push(ToolCall { id, name, args });
                    }
                    Delta::Done => break,
                    _ => {}
                }
            }
        }

        Ok(final_response)
    }
}
```

---

## 3. Request Builder

```rust
impl OpenAiCompatClient {
    fn build_request_body(&self, req: &LlmRequest, stream: bool) -> Value {
        let messages: Vec<Value> = req.messages.iter().map(|m| {
            match &m.role {
                Role::User => json!({ "role": "user", "content": m.content }),
                Role::Assistant => {
                    let mut msg = json!({ "role": "assistant", "content": m.content });
                    if !m.tool_calls.is_empty() {
                        msg["tool_calls"] = json!(
                            m.tool_calls.iter().map(|tc| json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.args.to_string()
                                }
                            })).collect::<Vec<_>>()
                        );
                    }
                    msg
                }
                Role::Tool { tool_use_id } => json!({
                    "role": "tool",
                    "tool_call_id": tool_use_id,
                    "content": m.content
                }),
                Role::System => json!({ "role": "system", "content": m.content }),
            }
        }).collect();

        let tools: Vec<Value> = req.tools.iter().map(|t| json!({
            "type": "function",
            "function": {
                "name": t.name,
                "description": t.description,
                "parameters": t.schema
            }
        })).collect();

        let mut body = json!({
            "model": req.model.as_deref().unwrap_or(&self.default_model),
            "messages": messages,
            "stream": stream,
            "max_tokens": req.max_tokens.unwrap_or(8096),
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools);
            body["tool_choice"] = json!("auto");
        }

        if let Some(temp) = req.temperature {
            body["temperature"] = json!(temp);
        }

        body
    }
}
```

---

## 4. Provider Configuration

```toml
# OpenAI
[llm.providers.openai]
type = "openai_compat"
base_url = "https://api.openai.com/v1"
api_key = "${env:OPENAI_API_KEY}"
model = "gpt-4o"

# OpenRouter (access 200+ models)
[llm.providers.openrouter]
type = "openai_compat"
base_url = "https://openrouter.ai/api/v1"
api_key = "${env:OPENROUTER_API_KEY}"
model = "anthropic/claude-3.5-sonnet"

[llm.providers.openrouter.headers]
"HTTP-Referer" = "https://github.com/yourname/talon"
"X-Title" = "Talon AI"

# Groq (fast Llama inference)
[llm.providers.groq]
type = "openai_compat"
base_url = "https://api.groq.com/openai/v1"
api_key = "${env:GROQ_API_KEY}"
model = "llama-3.3-70b-versatile"
```
---

## Related Documents

### Depends On
- [LLM Provider Abstraction](41_LLM_Provider_Abstraction.md)

### See Also
- [OpenAI Provider](42_OpenAI_Provider.md)
- [Streaming SSE Parser](44_Streaming_SSE_Parser.md)

