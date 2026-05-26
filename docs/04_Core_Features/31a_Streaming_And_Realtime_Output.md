# Streaming & Real-Time Output

> **Last corrected:** dogfood pass 2
>
> **Status:** ✅ Complete
> **Category:** Core Features

---

## 1. Overview

Talon streams LLM output from the first token rather than waiting for
the full response. This is critical for long responses and has two modes:

| Mode | Behavior |
|------|----------|
| **TUI** | Characters appear as they arrive in the terminal |
| **Telegram** | Message is edited every 1.5s with accumulated text |
| **HTTP API** | Server-Sent Events (SSE) per token |
| **CLI (non-TUI)** | Print chunks directly to stdout |

---

## 2. AgentEvent Enum

All streaming output flows through this enum:

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    // LLM is producing text
    TextDelta { content: String },
    TextComplete { content: String },

    // LLM requested a tool call
    ToolCallStart { id: String, name: String },
    ToolCallArgs { id: String, args_chunk: String },
    ToolCallComplete { id: String, name: String, args: Value },

    // Tool executed
    ToolResult { id: String, name: String, output: String, is_error: bool },

    // Approval needed
    ApprovalRequired { id: Uuid, tool: String, description: String, risk: String },
    ApprovalDecision { id: Uuid, approved: bool },

    // Agent loop
    IterationStart { n: u32 },
    Done { final_response: String, iterations: u32, usage: UsageSummary },
    Error { message: String, code: String },
}
```

---

## 3. Stream Pipeline

```
LLM HTTP SSE stream
   │
   ▼
SseStreamParser::feed() → Vec<Delta>
   │
   ▼
AgentLoop: Delta → AgentEvent
   │
   ▼
EventFanOut::broadcast()
   ├── TUI renderer (mpsc::Sender<AgentEvent>, capacity 256)
   ├── Telegram accumulator (capacity 32, drop-oldest)
   └── HTTP SSE sender (capacity 64)
```

---

## 4. Non-Streaming Fallback

Some providers (Ollama in some configurations) don't support streaming.
Talon detects this and falls back to a single complete response:

```rust
pub async fn complete_or_stream(
    &self,
    request: LlmRequest,
    event_tx: mpsc::Sender<AgentEvent>,
) -> Result<LlmResponse, LlmError> {
    if self.supports_streaming() {
        self.stream(request, event_tx).await
    } else {
        // No streaming: wait for full response, then emit as single event
        let response = self.complete(request).await?;
        event_tx.send(AgentEvent::TextComplete {
            content: response.content.clone()
        }).await.ok();
        Ok(response)
    }
}
```

---

## 5. Tool Output Streaming

Long-running terminal commands stream stdout lines as they're produced:

```rust
pub async fn execute_with_streaming(
    &self,
    command: &str,
    event_tx: mpsc::Sender<AgentEvent>,
    tool_call_id: &str,
) -> ToolResult {
    let child_result = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut child = match child_result {
        Ok(c) => c,
        Err(e) => return ToolResult::error(e.to_string()),
    };

    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout).lines();
    let mut output = String::new();
    let id = tool_call_id.to_string();

    while let Some(line) = reader.next_line().await.ok().flatten() {
        output.push_str(&line);
        output.push('\n');

        // Emit live output chunk
        event_tx.send(AgentEvent::ToolResult {
            id: id.clone(),
            name: "terminal".to_string(),
            output: line,
            is_error: false,
        }).await.ok();

        // Check size limit
        if output.len() > 50_000 {
            output.push_str("[truncated]");
            child.kill().await.ok();
            break;
        }
    }

    let status = child.wait().await.ok();
    let exit_code = status.and_then(|s| s.code()).unwrap_or(-1);

    if exit_code != 0 {
        ToolResult::error(format!("Exit {exit_code}\n{output}"))
    } else {
        ToolResult::success(output)
    }
}
```

---

## 6. HTTP SSE Endpoint

For clients consuming Talon via HTTP API:

```rust
async fn chat_stream_handler(
    State(app): State<Arc<AppState>>,
    Json(req): Json<ChatRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel::<AgentEvent>(64);

    tokio::spawn(async move {
        app.agent_loop()
            .run_with_events(req.message, req.session_id, tx)
            .await
            .ok();
    });

    let stream = ReceiverStream::new(rx).map(|event| {
        let data = serde_json::to_string(&event).unwrap_or_default();
        let event_type = match &event {
            AgentEvent::TextDelta { .. } => "text_delta",
            AgentEvent::ToolCallStart { .. } => "tool_call_start",
            AgentEvent::Done { .. } => "done",
            AgentEvent::Error { .. } => "error",
            _ => "event",
        };
        Ok(Event::default().event(event_type).data(data))
    });

    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
```

Client consumption (JavaScript):
```javascript
const es = new EventSource('/chat/stream', { method: 'POST', body: JSON.stringify({ message }) });
es.addEventListener('text_delta', e => {
    const { content } = JSON.parse(e.data);
    appendToChat(content);
});
es.addEventListener('done', e => es.close());
```
---

## Related Documents

### Depends On
- [LLM Provider Abstraction](../05_API_Bindings/41_LLM_Provider_Abstraction.md)

### See Also
- [Streaming SSE Parser](../05_API_Bindings/44_Streaming_SSE_Parser.md)
- [Stream Processing (Tokio)](../06_Concurrency/52_Stream_Processing.md)
- [Gateway Architecture](../02_Architecture/18_Gateway_MultiChannel_Architecture.md)

