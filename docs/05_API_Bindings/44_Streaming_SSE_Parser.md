# Streaming SSE Parser

> **Status:** ✅ Complete
> **Category:** API Bindings

---

## 1. Why a Custom Parser?

SSE (Server-Sent Events) streams from LLM APIs are deceptively complex:
- Chunks arrive mid-event, split across Bytes frames
- Events span multiple lines (`event:` + `data:` + blank separator)
- OpenAI and Anthropic have different event shapes
- Tool call arguments stream as partial JSON fragments that must be accumulated

A generic SSE parser handles the framing; provider-specific logic handles meaning.

---

## 2. Delta Type

The unified output type across all providers:

```rust
#[derive(Debug, Clone)]
pub enum Delta {
    /// Text content (streamed character by character or in chunks)
    Text(String),

    /// A complete tool call (assembled from streaming fragments)
    ToolCall(ToolCall),

    /// Extended thinking (Anthropic only)
    Thinking(String),

    /// Usage statistics (final event)
    Usage { input_tokens: u32, output_tokens: u32 },

    /// Stream is complete
    Done,
}
```

---

## 3. SseFrame Parser

Low-level framing: raw bytes → parsed SSE events.

```rust
pub struct SseFramer {
    buffer: String,
}

#[derive(Debug)]
pub struct SseEvent {
    pub event_type: Option<String>,
    pub data: String,
    pub id: Option<String>,
}

impl SseFramer {
    pub fn new() -> Self {
        Self { buffer: String::new() }
    }

    /// Feed raw bytes, get back complete events
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buffer.push_str(&String::from_utf8_lossy(bytes));

        let mut events = vec![];

        // Events are separated by double newline
        while let Some(pos) = self.buffer.find("\n\n") {
            let raw = self.buffer[..pos].to_string();
            self.buffer.drain(..pos + 2);

            if let Some(event) = Self::parse_event(&raw) {
                events.push(event);
            }
        }

        events
    }

    fn parse_event(raw: &str) -> Option<SseEvent> {
        let mut event_type = None;
        let mut data_lines = vec![];
        let mut id = None;

        for line in raw.lines() {
            if line.starts_with(':') {
                // SSE comment — skip
                continue;
            }
            if let Some(v) = line.strip_prefix("event: ") {
                event_type = Some(v.to_string());
            } else if let Some(v) = line.strip_prefix("data: ") {
                data_lines.push(v.to_string());
            } else if let Some(v) = line.strip_prefix("id: ") {
                id = Some(v.to_string());
            }
        }

        if data_lines.is_empty() {
            return None;
        }

        Some(SseEvent {
            event_type,
            data: data_lines.join("\n"),
            id,
        })
    }
}
```

---

## 4. OpenAI SSE Parser

```rust
pub fn parse_sse_stream(
    raw: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = Result<Delta, LlmError>> {
    let mut framer = SseFramer::new();
    // Track partial tool calls: index → PartialToolCall
    let mut partial_tools: HashMap<usize, PartialToolCall> = HashMap::new();

    raw.map_err(LlmError::Http)
       .flat_map(move |chunk_result| {
           let chunk = match chunk_result {
               Ok(c) => c,
               Err(e) => return futures::stream::iter(vec![Err(e)]),
           };

           let events = framer.feed(&chunk);
           let mut deltas: Vec<Result<Delta, LlmError>> = vec![];

           for event in events {
               if event.data == "[DONE]" {
                   // Flush any completed tool calls
                   for (_, tool) in partial_tools.drain() {
                       if let Ok(args) = serde_json::from_str(&tool.arguments_buffer) {
                           deltas.push(Ok(Delta::ToolCall(ToolCall {
                               id: tool.id,
                               name: tool.name,
                               arguments: args,
                           })));
                       }
                   }
                   deltas.push(Ok(Delta::Done));
                   continue;
               }

               match serde_json::from_str::<OpenAiChunk>(&event.data) {
                   Ok(chunk) => {
                       for choice in chunk.choices {
                           let d = &choice.delta;

                           if let Some(text) = &d.content {
                               if !text.is_empty() {
                                   deltas.push(Ok(Delta::Text(text.clone())));
                               }
                           }

                           for tc in &d.tool_calls {
                               let idx = tc.index as usize;
                               let entry = partial_tools.entry(idx).or_insert_with(|| {
                                   PartialToolCall {
                                       id: tc.id.clone().unwrap_or_default(),
                                       name: tc.function.name.clone().unwrap_or_default(),
                                       arguments_buffer: String::new(),
                                   }
                               });

                               if let Some(args) = &tc.function.arguments {
                                   entry.arguments_buffer.push_str(args);
                               }
                           }

                           if choice.finish_reason.as_deref() == Some("tool_calls") {
                               // All tool calls complete — emit them
                               for (_, tool) in partial_tools.drain() {
                                   let args = serde_json::from_str(&tool.arguments_buffer)
                                       .unwrap_or_default();
                                   deltas.push(Ok(Delta::ToolCall(ToolCall {
                                       id: tool.id,
                                       name: tool.name,
                                       arguments: args,
                                   })));
                               }
                           }
                       }

                       if let Some(usage) = chunk.usage {
                           deltas.push(Ok(Delta::Usage {
                               input_tokens: usage.prompt_tokens,
                               output_tokens: usage.completion_tokens,
                           }));
                       }
                   }
                   Err(e) => {
                       tracing::warn!(data = event.data, error = %e, "Failed to parse SSE chunk");
                   }
               }
           }

           futures::stream::iter(deltas)
       })
}
```

---

## 5. Wire Format Deserialization Types

```rust
#[derive(Debug, Deserialize)]
struct OpenAiChunk {
    choices: Vec<OpenAiChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    delta: OpenAiDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenAiDelta {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<OpenAiToolCallDelta>,
}

#[derive(Debug, Deserialize)]
struct OpenAiToolCallDelta {
    index: u32,
    id: Option<String>,
    function: OpenAiToolCallFunction,
}

#[derive(Debug, Deserialize, Default)]
struct OpenAiToolCallFunction {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

struct PartialToolCall {
    id: String,
    name: String,
    arguments_buffer: String,
}
```

---

## 6. Stream Combinators — Timeout & Rate

```rust
pub fn with_stream_timeout<S>(
    stream: S,
    token_timeout: Duration,
) -> impl Stream<Item = Result<Delta, LlmError>>
where
    S: Stream<Item = Result<Delta, LlmError>> + Send + 'static,
{
    // Timeout resets on every token — catches stalled streams
    stream.timeout(token_timeout)
        .map(|r| match r {
            Ok(item) => item,
            Err(_) => Err(LlmError::StreamTimeout),
        })
}
```

---

## 7. Assembling the Full Response

```rust
pub async fn collect_stream(
    mut stream: BoxStream<'static, Result<Delta, LlmError>>,
) -> Result<AssembledResponse, LlmError> {
    let mut text = String::new();
    let mut tool_calls = vec![];
    let mut usage = None;

    while let Some(delta) = stream.next().await {
        match delta? {
            Delta::Text(t) => text.push_str(&t),
            Delta::ToolCall(tc) => tool_calls.push(tc),
            Delta::Usage { input_tokens, output_tokens } => {
                usage = Some(TokenUsage { input_tokens, output_tokens });
            }
            Delta::Done => break,
            Delta::Thinking(_) => {} // not stored by default
        }
    }

    Ok(AssembledResponse { text, tool_calls, usage })
}
```
---

## Related Documents

### Depends On
- [LLM Provider Abstraction](41_LLM_Provider_Abstraction.md)

### See Also
- [Stream Processing (Tokio)](../06_Concurrency/52_Stream_Processing.md)
- [Streaming & Realtime Output](../04_Core_Features/31a_Streaming_And_Realtime_Output.md)

