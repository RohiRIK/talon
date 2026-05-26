# Stream Processing

> **Status:** ✅ Complete
> **Category:** Concurrency

---

## 1. Overview

Talon has three distinct streaming scenarios:

| Scenario | Source | Consumers |
|----------|--------|-----------|
| LLM response stream | HTTP SSE from provider | TUI, Telegram, HTTP gateway |
| Tool output stream | Long-running subprocess stdout | TUI live output |
| Agent event stream | Internal `AgentEvent` channel | All gateways simultaneously |

All three use Tokio `mpsc` channels and async streams under the hood.

---

## 2. LLM SSE Stream Processing

The core challenge: Anthropic and OpenAI send SSE events that need to be
parsed incrementally and assembled into tool calls + text chunks.

```rust
// talon-llm/src/stream.rs

pub struct SseStreamParser {
    buffer: String,
    pending_tool_calls: HashMap<String, PartialToolCall>,
}

#[derive(Debug)]
pub struct PartialToolCall {
    pub id: String,
    pub name: String,
    pub args_buf: String,
}

impl SseStreamParser {
    pub fn feed(&mut self, raw_bytes: &[u8]) -> Vec<Delta> {
        self.buffer.push_str(std::str::from_utf8(raw_bytes).unwrap_or(""));
        let mut deltas = vec![];

        // Process complete SSE events (delimited by \n\n)
        while let Some(pos) = self.buffer.find("\n\n") {
            let event = self.buffer[..pos].to_string();
            self.buffer.drain(..pos + 2);

            if let Some(delta) = self.parse_event(&event) {
                deltas.push(delta);
            }
        }

        deltas
    }

    fn parse_event(&mut self, event: &str) -> Option<Delta> {
        let data = event.lines()
            .find(|l| l.starts_with("data: "))?
            .trim_start_matches("data: ");

        if data == "[DONE]" { return Some(Delta::Done); }

        let json: Value = serde_json::from_str(data).ok()?;
        self.route_event(json)
    }

    fn route_event(&mut self, json: Value) -> Option<Delta> {
        // Anthropic event format
        match json.get("type")?.as_str()? {
            "content_block_delta" => {
                let delta = json.pointer("/delta")?;
                match delta.get("type")?.as_str()? {
                    "text_delta" => {
                        Some(Delta::Text(delta.get("text")?.as_str()?.to_string()))
                    }
                    "input_json_delta" => {
                        let id = json.pointer("/index")?.to_string();
                        let chunk = delta.get("partial_json")?.as_str()?.to_string();
                        if let Some(tc) = self.pending_tool_calls.get_mut(&id) {
                            tc.args_buf.push_str(&chunk);
                        }
                        Some(Delta::ToolUseChunk { id, chunk })
                    }
                    _ => None,
                }
            }
            "content_block_start" => {
                let block = json.get("content_block")?;
                if block.get("type")?.as_str()? == "tool_use" {
                    let id = block.get("id")?.as_str()?.to_string();
                    let name = block.get("name")?.as_str()?.to_string();
                    self.pending_tool_calls.insert(id.clone(), PartialToolCall {
                        id: id.clone(), name: name.clone(), args_buf: String::new(),
                    });
                    return Some(Delta::ToolUseStart { id, name });
                }
                None
            }
            "message_delta" => {
                if let Some(stop) = json.pointer("/delta/stop_reason").and_then(|v| v.as_str()) {
                    return Some(Delta::StopReason(stop.parse().unwrap_or(StopReason::EndTurn)));
                }
                None
            }
            "message_start" => {
                let usage = json.pointer("/message/usage");
                if let Some(u) = usage {
                    return Some(Delta::Usage(Usage {
                        input_tokens: u.get("input_tokens")
                            .and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                        output_tokens: u.get("output_tokens")
                            .and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    }));
                }
                None
            }
            _ => None,
        }
    }
}
```

---

## 3. Subprocess Output Streaming

Long-running terminal commands need live stdout delivery:

```rust
pub async fn stream_subprocess(
    mut child: tokio::process::Child,
    tx: mpsc::Sender<String>,
    max_bytes: usize,
) -> Result<std::process::ExitStatus, ToolError> {
    let stdout = child.stdout.take().expect("stdout not captured");
    let mut reader = BufReader::new(stdout).lines();
    let mut total_bytes = 0usize;
    let mut truncated = false;

    loop {
        tokio::select! {
            line = reader.next_line() => {
                match line? {
                    Some(l) => {
                        total_bytes += l.len() + 1;
                        if total_bytes > max_bytes {
                            if !truncated {
                                let _ = tx.send("[Output truncated — max size reached]".into()).await;
                                truncated = true;
                            }
                            // Still drain stdout to prevent blocking child
                        } else {
                            let _ = tx.send(l).await;
                        }
                    }
                    None => break,  // EOF
                }
            }
        }
    }

    let status = child.wait().await?;
    Ok(status)
}
```

---

## 4. Agent Event Fan-Out

One agent run → multiple simultaneous consumers (TUI + Telegram + log file):

```rust
pub struct EventFanOut {
    senders: Vec<mpsc::Sender<AgentEvent>>,
}

impl EventFanOut {
    pub async fn broadcast(&self, event: AgentEvent) {
        // Clone event to all consumers
        let futures: Vec<_> = self.senders.iter().map(|tx| {
            let e = event.clone();
            async move { tx.send(e).await.ok(); }
        }).collect();
        futures::future::join_all(futures).await;
    }
}
```

Usage in the agent loop:

```rust
// Instead of a single output_tx, use fan-out
let fan_out = EventFanOut::new(vec![
    tui_tx,        // → TUI renderer
    telegram_tx,   // → Telegram delivery
    log_tx,        // → file logger
]);

// In loop:
fan_out.broadcast(AgentEvent::TextDelta(chunk)).await;
```

---

## 5. Backpressure Strategy

If a slow consumer (e.g., Telegram API rate-limited) can't keep up with the
agent's event stream, we need [backpressure](53_Resource_Limits_And_Backpressure.md). Talon's strategy:

| Consumer | Buffer size | On full |
|----------|-------------|---------|
| TUI | 256 events | Block (TUI is fast) |
| Telegram | 32 events | Drop oldest (prevent delay) |
| Log file | 1024 events | Block (disk is fast) |

```rust
// Telegram sender: ring-buffer semantics
pub struct TelegramEventQueue {
    inner: Arc<Mutex<VecDeque<AgentEvent>>>,
    max_size: usize,
}

impl TelegramEventQueue {
    pub fn push(&self, event: AgentEvent) {
        let mut q = self.inner.lock().unwrap();
        if q.len() >= self.max_size {
            q.pop_front();  // drop oldest
        }
        q.push_back(event);
    }
}
```

---

## 6. Streaming to Telegram: Chunked Messages

Telegram has a 4096 character message limit and doesn't support true streaming.
Talon buffers LLM text and sends updates:

```rust
pub struct TelegramStreamAccumulator {
    bot: Bot,
    chat_id: ChatId,
    message_id: Option<MessageId>,
    buffer: String,
    last_edit: Instant,
    edit_interval: Duration,  // default: 1.5s
}

impl TelegramStreamAccumulator {
    pub async fn push_chunk(&mut self, chunk: &str) {
        self.buffer.push_str(chunk);

        // Edit existing message if possible, else send new
        if self.last_edit.elapsed() >= self.edit_interval {
            self.flush().await;
        }
    }

    pub async fn flush(&mut self) {
        if self.buffer.is_empty() { return; }

        let text = self.buffer.clone();
        match self.message_id {
            Some(id) => {
                self.bot.edit_message_text(self.chat_id, id, &text)
                    .parse_mode(ParseMode::MarkdownV2)
                    .await.ok();
            }
            None => {
                match self.bot.send_message(self.chat_id, &text)
                    .parse_mode(ParseMode::MarkdownV2)
                    .await
                {
                    Ok(msg) => { self.message_id = Some(msg.id); }
                    Err(e) => { tracing::warn!("Telegram send failed: {e}"); }
                }
            }
        }
        self.last_edit = Instant::now();
    }
}
```
---

## Related Documents

### Depends On
- [Channel Patterns](51_Channel_Patterns.md)

### See Also
- [Streaming SSE Parser](../05_API_Bindings/44_Streaming_SSE_Parser.md)
- [Streaming & Realtime Output](../04_Core_Features/31a_Streaming_And_Realtime_Output.md)

