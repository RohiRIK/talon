# Agent Loop Implementation

> **Last corrected:** dogfood pass 4

> **Status:** ✅ Complete
> **Category:** Core Features

---

## 1. The Loop in One Diagram

```
User input
    │
    ▼
┌─────────────────────────────────────────────────────┐
│  AgentLoop::run()                                   │
│                                                     │
│  messages: Vec<Message>   ←──── initial input       │
│                                                     │
│  loop {                                             │
│    ┌──────────────┐                                 │
│    │  LLM Call    │  ← full history + tools         │
│    └──────┬───────┘                                 │
│           │ streaming response                      │
│           ▼                                         │
│    ┌──────────────┐                                 │
│    │ Parse delta  │  text chunk | tool_use block    │
│    └──────┬───────┘                                 │
│           │                                         │
│    ┌──────┴───────────────────────────┐             │
│    │ text delta?  → stream to user   │             │
│    │ tool_use?    → approval gate    │             │
│    │              → execute tool     │             │
│    │              → append result    │             │
│    │ stop_reason: end_turn → break   │             │
│    └─────────────────────────────────┘             │
│  }                                                  │
│                                                     │
│  return final_response                              │
└─────────────────────────────────────────────────────┘
```

---

## 2. Core Types

```rust
// talon-core/src/agent/loop.rs

pub struct AgentLoop {
    config: AgentConfig,
    llm: Arc<dyn LlmProvider>,
    tools: Arc<ToolRegistry>,
    memory: Arc<MemoryStore>,
    output_tx: mpsc::Sender<AgentEvent>,
    cancel: CancellationToken,
}

pub enum AgentEvent {
    ThinkingStarted,
    ToolCallStarted { tool_name: String, call_id: String },
    ToolCallCompleted { call_id: String, result: ToolResult },
    AssistantMessage(String),
    FinalResponse(String),
    Error(AgentError),
}

pub struct AgentRunResult {
    pub final_response: String,
    pub tool_call_count: usize,
    pub iterations: u32,
    pub session_id: Uuid,
}
```

---

## 3. Full Loop Implementation

```rust
impl AgentLoop {
    pub async fn run(
        &mut self,
        mut messages: Vec<Message>,
        session_id: Uuid,
    ) -> Result<AgentRunResult, AgentError> {

        let mut iterations = 0u32;
        let mut tool_call_count = 0usize;
        let mut final_response = String::new();

        loop {
            // Safety ceiling
            if iterations >= self.config.max_iterations {
                return Err(AgentError::MaxIterationsExceeded(iterations));
            }

            // Cancellation check
            if self.cancel.is_cancelled() {
                return Err(AgentError::Cancelled);
            }

            iterations += 1;

            // Trim context if approaching limit
            let context_messages = self.trim_context(&messages)?;

            // Build request
            let request = LlmRequest {
                model: self.config.default_model.clone(),
                messages: context_messages,
                system: self.config.system_prompt.clone(),
                tools: self.tools.schemas(),
                max_tokens: self.config.max_tokens,
                stream: true,
            };

            // Call LLM — get event stream
            let mut stream = tokio::select! {
                result = self.llm.complete(request) => result?,
                _ = self.cancel.cancelled() => return Err(AgentError::Cancelled),
            };

            // Collect assistant turn
            let mut text_buf = String::new();
            let mut tool_calls: Vec<ToolCallRequest> = vec![];
            let mut stop_reason = StopReason::EndTurn;

            while let Some(delta) = tokio::select! {
                d = stream.next() => d,
                _ = self.cancel.cancelled() => None,
            } {
                match delta? {
                    Delta::Text(chunk) => {
                        text_buf.push_str(&chunk);
                        let _ = self.output_tx.send(AgentEvent::AssistantMessage(chunk.clone())).await;
                    }
                    Delta::ToolUseStart { id, name } => {
                        tool_calls.push(ToolCallRequest { id: id.clone(), name: name.clone(), args_buf: String::new() });
                        let _ = self.output_tx.send(AgentEvent::ToolCallStarted {
                            tool_name: name,
                            call_id: id,
                        }).await;
                    }
                    Delta::ToolUseChunk { id, chunk } => {
                        if let Some(tc) = tool_calls.iter_mut().find(|t| t.id == id) {
                            tc.args_buf.push_str(&chunk);
                        }
                    }
                    Delta::StopReason(r) => { stop_reason = r; }
                    Delta::Usage(u) => {
                        tracing::debug!(input=u.input_tokens, output=u.output_tokens, "token usage");
                    }
                }
            }

            // Append assistant message
            if !text_buf.is_empty() || !tool_calls.is_empty() {
                let assistant_msg = Message::assistant_with_tools(&text_buf, &tool_calls);
                messages.push(assistant_msg.clone());
                final_response = text_buf.clone();

                // Persist to DB
                self.memory.append_message(session_id, &assistant_msg).await
                    .unwrap_or_else(|e| tracing::warn!("Failed to persist message: {e}"));
            }

            // If no tool calls or end_turn, we're done
            if tool_calls.is_empty() || stop_reason == StopReason::EndTurn {
                break;
            }

            // Execute all tool calls
            let tool_results = self.execute_tool_calls(&tool_calls).await;
            tool_call_count += tool_results.len();

            // Append tool results as a user message
            let results_msg = Message::tool_results(&tool_results);
            messages.push(results_msg.clone());

            self.memory.append_message(session_id, &results_msg).await
                .unwrap_or_else(|e| tracing::warn!("Failed to persist tool results: {e}"));
        }

        let _ = self.output_tx.send(AgentEvent::FinalResponse(final_response.clone())).await;

        Ok(AgentRunResult {
            final_response,
            tool_call_count,
            iterations,
            session_id,
        })
    }

    async fn execute_tool_calls(
        &self,
        calls: &[ToolCallRequest],
    ) -> Vec<ToolResult> {
        // Run all tool calls concurrently (within the same iteration)
        let futures: Vec<_> = calls.iter().map(|tc| {
            let tools = self.tools.clone();
            let output_tx = self.output_tx.clone();
            let tc = tc.clone();

            async move {
                let args: Value = serde_json::from_str(&tc.args_buf)
                    .unwrap_or(Value::Object(Default::default()));

                let _ = output_tx.send(AgentEvent::ToolCallStarted {
                    tool_name: tc.name.clone(),
                    call_id: tc.id.clone(),
                }).await;

                let result = tools.execute(&tc.name, args, &ToolContext::default()).await;

                let (output, is_error) = match result {
                    Ok(o) => (o.content, false),
                    Err(e) => (e.to_string(), true),
                };

                let tool_result = ToolResult { id: tc.id.clone(), output: output.clone(), is_error };

                let _ = output_tx.send(AgentEvent::ToolCallCompleted {
                    call_id: tc.id.clone(),
                    result: tool_result.clone(),
                }).await;

                tool_result
            }
        }).collect();

        futures::future::join_all(futures).await
    }

    fn trim_context(&self, messages: &[Message]) -> Result<Vec<Message>, AgentError> {
        // Naive sliding window — keep last N messages
        // TODO: token-aware trimming with tiktoken-rs
        let max = self.config.context_window_messages.unwrap_or(100);
        if messages.len() <= max {
            return Ok(messages.to_vec());
        }

        // Always keep system context; drop oldest user/assistant pairs
        let keep_from = messages.len() - max;
        Ok(messages[keep_from..].to_vec())
    }
}
```

---

## 4. Tool Call Concurrency

Multiple tool calls in a single LLM response run **concurrently**:

```
LLM response: [tool_use: web_search], [tool_use: read_file], [tool_use: memory]
                     │                        │                     │
                     └────────── join_all ────┘─────────────────────┘
                                     │
                              all results ready
                                     │
                              tool_results message appended
```

This is a significant improvement over sequential Python `await` chains —
independent tool calls don't wait for each other.

---

## 5. Context Window Pressure

Talon tracks approximate token usage per message:

```rust
impl Message {
    pub fn approx_tokens(&self) -> usize {
        // ~4 chars per token (rough heuristic)
        // Replace with tiktoken-rs for accuracy
        match &self.content {
            MessageContent::Text(s) => s.len() / 4,
            MessageContent::Blocks(blocks) => blocks.iter()
                .map(|b| b.approx_tokens())
                .sum(),
        }
    }
}

fn trim_to_token_budget(
    messages: &[Message],
    budget: usize,
) -> Vec<Message> {
    let mut total = 0usize;
    let mut result: Vec<Message> = vec![];

    for msg in messages.iter().rev() {
        let tokens = msg.approx_tokens();
        if total + tokens > budget { break; }
        total += tokens;
        result.push(msg.clone());
    }

    result.reverse();
    result
}
```

---

## 6. Iteration Counter & Safety

```rust
// AgentConfig
pub struct AgentConfig {
    pub max_iterations: u32,   // default: 50
    pub max_tokens: u32,       // per LLM call, default: 8192
    pub context_window_messages: Option<usize>,  // default: 100
}

// Error
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Max iterations ({0}) exceeded — agent stopped")]
    MaxIterationsExceeded(u32),

    #[error("Agent cancelled by user")]
    Cancelled,

    #[error("LLM error: {0}")]
    Llm(#[from] LlmError),

    #[error("Context too large to fit in model window")]
    ContextOverflow,
}
```
---

## Related Documents

### Depends On
- [Core Agent Loop Design](../02_Architecture/13_Core_Agent_Loop_Design.md)
- [Tool Execution Engine](30_Tool_Execution_Engine.md)
- [LLM Provider Abstraction](../05_API_Bindings/41_LLM_Provider_Abstraction.md)

### See Also
- [State Machine & Lifecycle](../02_Architecture/14_State_Machine_And_Lifecycle.md)
- [Streaming & Realtime Output](31a_Streaming_And_Realtime_Output.md)

