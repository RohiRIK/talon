# Anthropic Provider (Native)

> **Status:** ✅ Complete
> **Category:** API Bindings

---

## 1. Why a Native Anthropic Client?

Anthropic's API has important differences from OpenAI-compat:

| Feature | OpenAI | Anthropic |
|---------|--------|-----------|
| Tool schema field | `parameters` | `input_schema` |
| Tool results | `role: tool` message | `tool_result` content block in `user` message |
| System prompt | First message `role: system` | Top-level `system` field |
| Token counting | Not native | `/v1/messages/count_tokens` endpoint |
| Multi-turn tool use | Tool message per result | Content blocks in user turn |
| Thinking tokens | Not supported | `thinking` delta type |

A native client handles all these correctly without workarounds.

---

## 2. AnthropicClient Struct

```rust
pub struct AnthropicClient {
    client: reqwest::Client,
    api_key: SecretString,
    model: String,
    base_url: String,
    beta_headers: Vec<String>,
}

impl AnthropicClient {
    pub fn new(config: &AnthropicConfig) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap(),
            api_key: SecretString::new(config.api_key.clone()),
            model: config.model.clone(),
            base_url: "https://api.anthropic.com".into(),
            beta_headers: config.beta_features.clone().unwrap_or_default(),
        }
    }
}
```

---

## 3. Message Conversion

The key complexity: Talon's unified `Message` type must serialize to Anthropic's format.

```rust
fn to_anthropic_messages(
    messages: &[Message],
) -> (Option<String>, Vec<serde_json::Value>) {
    let mut system = None;
    let mut out = vec![];

    for msg in messages {
        match msg.role {
            Role::System => {
                // Anthropic takes system as top-level field, not in messages array
                system = Some(match &msg.content {
                    MessageContent::Text(t) => t.clone(),
                    MessageContent::Blocks(b) => blocks_to_text(b),
                });
            }
            Role::User => {
                out.push(serde_json::json!({
                    "role": "user",
                    "content": message_content_to_anthropic(&msg.content),
                }));
            }
            Role::Assistant => {
                out.push(serde_json::json!({
                    "role": "assistant",
                    "content": message_content_to_anthropic(&msg.content),
                }));
            }
            Role::Tool => {
                // Tool results attach to the last user turn or create a new one
                let tool_result = serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": msg.tool_call_id,
                    "content": match &msg.content {
                        MessageContent::Text(t) => serde_json::json!(t),
                        MessageContent::Blocks(b) => serde_json::json!(b),
                    }
                });

                // Merge into previous user turn if it exists
                if let Some(last) = out.last_mut() {
                    if last["role"] == "user" {
                        if let Some(arr) = last["content"].as_array_mut() {
                            arr.push(tool_result);
                            continue;
                        }
                    }
                }

                // Otherwise start a new user turn
                out.push(serde_json::json!({
                    "role": "user",
                    "content": [tool_result],
                }));
            }
        }
    }

    (system, out)
}
```

---

## 4. Tool Definition Serialization

```rust
fn tool_to_anthropic(tool: &ToolDefinition) -> serde_json::Value {
    serde_json::json!({
        "name": tool.name,
        "description": tool.description,
        "input_schema": tool.parameters,  // note: NOT "parameters"
    })
}
```

---

## 5. Streaming Response Parser

```rust
#[async_trait]
impl LlmProvider for AnthropicClient {
    async fn complete(
        &self,
        req: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<Delta, LlmError>>, LlmError> {
        let (system, messages) = to_anthropic_messages(&req.messages);

        let mut body = serde_json::json!({
            "model": req.model.as_deref().unwrap_or(&self.model),
            "messages": messages,
            "max_tokens": req.max_tokens.unwrap_or(8192),
            "stream": true,
        });

        if let Some(sys) = system {
            body["system"] = serde_json::json!(sys);
        }

        if !req.tools.is_empty() {
            body["tools"] = serde_json::json!(
                req.tools.iter().map(tool_to_anthropic).collect::<Vec<_>>()
            );
        }

        let mut request = self.client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", self.api_key.expose_secret())
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");

        for beta in &self.beta_headers {
            request = request.header("anthropic-beta", beta);
        }

        let resp = request.json(&body).send().await.map_err(LlmError::Http)?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(LlmError::ApiError { status, body });
        }

        Ok(Box::pin(parse_anthropic_sse(resp.bytes_stream())))
    }
}
```

---

## 6. Anthropic SSE Event Parser

Anthropic SSE events are more complex than OpenAI's:

```rust
fn parse_anthropic_sse(
    raw: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = Result<Delta, LlmError>> {
    let mut buf = String::new();
    let mut current_tool: Option<PartialToolCall> = None;

    raw.map_err(LlmError::Http)
        .flat_map(move |bytes_result| {
            let bytes = match bytes_result {
                Ok(b) => b,
                Err(e) => return futures::stream::iter(vec![Err(e)]),
            };

            buf.push_str(&String::from_utf8_lossy(&bytes));
            let mut deltas = vec![];

            while let Some(pos) = buf.find("\n\n") {
                let event_str = buf[..pos].to_string();
                buf.drain(..pos + 2);

                if let Ok(delta) = parse_anthropic_event(&event_str, &mut current_tool) {
                    if let Some(d) = delta {
                        deltas.push(Ok(d));
                    }
                }
            }

            futures::stream::iter(deltas)
        })
}

fn parse_anthropic_event(
    event: &str,
    partial_tool: &mut Option<PartialToolCall>,
) -> Result<Option<Delta>, LlmError> {
    let mut event_type = None;
    let mut data = None;

    for line in event.lines() {
        if let Some(t) = line.strip_prefix("event: ") { event_type = Some(t.to_string()); }
        if let Some(d) = line.strip_prefix("data: ")  { data = Some(d.to_string()); }
    }

    let data_str = match data { Some(d) => d, None => return Ok(None) };
    let json: serde_json::Value = serde_json::from_str(&data_str)?;

    match event_type.as_deref() {
        Some("content_block_delta") => {
            let delta = &json["delta"];
            match delta["type"].as_str() {
                Some("text_delta") => Ok(Some(Delta::Text(
                    delta["text"].as_str().unwrap_or("").to_string()
                ))),
                Some("input_json_delta") => {
                    // Accumulate tool arguments (streamed as partial JSON)
                    if let Some(tool) = partial_tool.as_mut() {
                        tool.arguments_buffer.push_str(
                            delta["partial_json"].as_str().unwrap_or("")
                        );
                    }
                    Ok(None)
                }
                Some("thinking_delta") => Ok(Some(Delta::Thinking(
                    delta["thinking"].as_str().unwrap_or("").to_string()
                ))),
                _ => Ok(None),
            }
        }
        Some("content_block_start") => {
            let block = &json["content_block"];
            if block["type"].as_str() == Some("tool_use") {
                *partial_tool = Some(PartialToolCall {
                    id: block["id"].as_str().unwrap_or("").to_string(),
                    name: block["name"].as_str().unwrap_or("").to_string(),
                    arguments_buffer: String::new(),
                });
            }
            Ok(None)
        }
        Some("content_block_stop") => {
            if let Some(tool) = partial_tool.take() {
                let arguments = serde_json::from_str(&tool.arguments_buffer)
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                return Ok(Some(Delta::ToolCall(ToolCall {
                    id: tool.id,
                    name: tool.name,
                    arguments,
                })));
            }
            Ok(None)
        }
        Some("message_stop") | Some("message_delta") => Ok(Some(Delta::Done)),
        _ => Ok(None),
    }
}
```

---

## 7. Token Counting

```rust
impl AnthropicClient {
    pub async fn count_tokens_api(&self, req: &CompletionRequest) -> Result<u32, LlmError> {
        let (system, messages) = to_anthropic_messages(&req.messages);
        let mut body = serde_json::json!({ "model": self.model, "messages": messages });
        if let Some(s) = system { body["system"] = serde_json::json!(s); }

        let resp: serde_json::Value = self.client
            .post(format!("{}/v1/messages/count_tokens", self.base_url))
            .header("x-api-key", self.api_key.expose_secret())
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "token-counting-2024-11-01")
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        Ok(resp["input_tokens"].as_u64().unwrap_or(0) as u32)
    }
}
```
---

## Related Documents

### Depends On
- [LLM Provider Abstraction](41_LLM_Provider_Abstraction.md)

### See Also
- [OpenAI Provider](42_OpenAI_Provider.md)
- [Streaming SSE Parser](44_Streaming_SSE_Parser.md)

