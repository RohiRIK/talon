# Anthropic API Integration

> **Status:** ✅ Complete
> **Category:** API Bindings

---

## 1. Why a Separate Anthropic Client?

Anthropic's Messages API differs from OpenAI's format in important ways:
- Tool use: `tool_use` content blocks (not `tool_calls` in message)
- Tool results: role `user` with `tool_result` content type (not role `tool`)
- System prompt: separate top-level `system` field (not in messages array)
- Streaming: different SSE event structure (`content_block_delta`)

These differences require a dedicated implementation.

---

## 2. Client Implementation

```rust
pub struct AnthropicClient {
    http: reqwest::Client,
    api_key: String,
    default_model: String,
    beta_headers: Vec<String>,  // e.g., ["interleaved-thinking-2025-05-14"]
}

#[async_trait]
impl LlmProvider for AnthropicClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let body = self.build_body(&request);
        let resp: AnthropicResponse = self.http
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send().await?
            .error_for_status()?
            .json().await?;

        self.parse_response(resp)
    }

    async fn stream(
        &self,
        request: LlmRequest,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<LlmResponse, LlmError> {
        let mut body = self.build_body(&request);
        body["stream"] = json!(true);

        let resp = self.http
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send().await?
            .error_for_status()?;

        let mut stream = resp.bytes_stream();
        let mut parser = AnthropicSseParser::new();
        let mut final_text = String::new();
        let mut tool_calls = vec![];

        while let Some(chunk) = stream.next().await {
            for delta in parser.feed(&chunk?) {
                match delta {
                    Delta::Text(t) => {
                        event_tx.send(AgentEvent::TextDelta { content: t.clone() }).await.ok();
                        final_text.push_str(&t);
                    }
                    Delta::ToolUseComplete { id, name, args } => {
                        let args: Value = serde_json::from_str(&args).unwrap_or(Value::Null);
                        tool_calls.push(ToolCall { id, name, args });
                    }
                    Delta::Done => {}
                    _ => {}
                }
            }
        }

        Ok(LlmResponse {
            content: final_text,
            tool_calls,
            stop_reason: StopReason::EndTurn,
            usage: None,
        })
    }
}
```

---

## 3. Request Body Builder

```rust
impl AnthropicClient {
    fn build_body(&self, req: &LlmRequest) -> Value {
        // Separate system from conversation
        let (system_msgs, conv_msgs): (Vec<_>, Vec<_>) = req.messages.iter()
            .partition(|m| m.role == Role::System);

        let system = system_msgs.first().map(|m| m.content.as_str()).unwrap_or("");

        // Convert messages to Anthropic format
        let messages: Vec<Value> = conv_msgs.iter().map(|m| {
            match &m.role {
                Role::User => json!({ "role": "user", "content": m.content }),
                Role::Assistant if m.tool_calls.is_empty() => {
                    json!({ "role": "assistant", "content": m.content })
                }
                Role::Assistant => {
                    // Assistant with tool calls: content is array of blocks
                    let mut content = vec![];
                    if !m.content.is_empty() {
                        content.push(json!({ "type": "text", "text": m.content }));
                    }
                    for tc in &m.tool_calls {
                        content.push(json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": tc.args
                        }));
                    }
                    json!({ "role": "assistant", "content": content })
                }
                Role::Tool { tool_use_id } => {
                    json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": m.content
                        }]
                    })
                }
                Role::System => unreachable!(),
            }
        }).collect();

        let tools: Vec<Value> = req.tools.iter().map(|t| json!({
            "name": t.name,
            "description": t.description,
            "input_schema": t.schema
        })).collect();

        let mut body = json!({
            "model": req.model.as_deref().unwrap_or(&self.default_model),
            "system": system,
            "messages": messages,
            "max_tokens": req.max_tokens.unwrap_or(8096),
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        // Extended thinking (claude-3-7+)
        if let Some(thinking_budget) = req.thinking_budget_tokens {
            body["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": thinking_budget
            });
        }

        body
    }
}
```

---

## 4. Configuration

```toml
[llm.providers.anthropic]
type = "anthropic"
api_key = "${env:ANTHROPIC_API_KEY}"
model = "claude-sonnet-4-5"

# Optional: Claude with extended thinking
[llm.providers.anthropic_thinking]
type = "anthropic"
api_key = "${env:ANTHROPIC_API_KEY}"
model = "claude-3-7-sonnet-20250219"
thinking_budget_tokens = 10000
```
---

## Related Documents

### Depends On
- [LLM Provider Abstraction](41_LLM_Provider_Abstraction.md)

### See Also
- [Anthropic Provider](43_Anthropic_Provider.md)

