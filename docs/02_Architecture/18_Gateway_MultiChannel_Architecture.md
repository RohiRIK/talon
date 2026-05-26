# Gateway & Multi-Channel Architecture

> **Status:** ✅ Complete
> **Category:** Architecture

---

## 1. Design Principle

The gateway layer is **thin**. Its only jobs are:
1. Receive input from a channel (Telegram message, HTTP POST, CLI keystroke)
2. Convert it to `AgentInput`
3. Forward `AgentEvent` stream back to that channel

All business logic lives in `talon-core`. Gateways are interchangeable.

---

## 2. Architecture Diagram

```
                    ┌─────────────────────────────────────┐
                    │         talon-gateway              │
                    │                                     │
  Telegram ─────►  │  TelegramGateway                    │
  Discord  ─────►  │  DiscordGateway    ──► AgentInput   │
  CLI TUI  ─────►  │  CliGateway                │        │
  HTTP API ─────►  │  HttpGateway               ▼        │
                    │                    AgentRouter      │
                    └──────────────────────┬──────────────┘
                                           │
                                           ▼
                              ┌────────────────────────┐
                              │      talon-core       │
                              │   AgentLoop::run()     │
                              └────────────────────────┘
                                           │
                                    AgentEvent stream
                                           │
                    ┌──────────────────────┘
                    ▼
              GatewayRouter (mpsc broadcast)
            ├── TelegramGateway::send()
            ├── DiscordGateway::send()
            └── CliGateway::print()
```

---

## 3. Core Gateway Trait

```rust
// talon-gateway/src/lib.rs

#[async_trait]
pub trait Gateway: Send + Sync {
    fn name(&self) -> &str;

    /// Start listening for incoming messages.
    /// Each incoming message produces an AgentInput sent on tx.
    async fn listen(&self, tx: mpsc::Sender<AgentInput>) -> Result<(), GatewayError>;

    /// Deliver an event (text chunk, tool output, final message) to this channel.
    async fn deliver(&self, event: &DeliveryEvent) -> Result<(), GatewayError>;
}

pub struct AgentInput {
    pub session_id: Uuid,
    pub user_id: String,
    pub platform: String,           // "telegram", "discord", "cli", "http"
    pub chat_id: String,
    pub thread_id: Option<String>,
    pub content: InputContent,
    pub reply_fn: Arc<dyn Fn(DeliveryEvent) + Send + Sync>,
}

pub enum InputContent {
    Text(String),
    Voice { file_path: PathBuf, duration_secs: u32 },
    Media { file_path: PathBuf, caption: Option<String> },
}

pub enum DeliveryEvent {
    TextChunk(String),              // streaming partial output
    FinalMessage(String),           // complete message
    MediaFile { path: PathBuf, caption: Option<String> },
    ApprovalRequest { tool: String, description: String, id: Uuid },
    ApprovalResult { id: Uuid, approved: bool },
    Error(String),
}
```

---

## 4. Telegram Gateway

```rust
pub struct TelegramGateway {
    bot: Bot,
    home_chat_id: ChatId,
}

#[async_trait]
impl Gateway for TelegramGateway {
    fn name(&self) -> &str { "telegram" }

    async fn listen(&self, tx: mpsc::Sender<AgentInput>) -> Result<(), GatewayError> {
        let handler = dptree::entry()
            .branch(Update::filter_message().endpoint(
                |bot: Bot, msg: Message, tx: mpsc::Sender<AgentInput>| async move {
                    if let Some(text) = msg.text() {
                        let _ = tx.send(AgentInput {
                            session_id: Uuid::new_v4(),
                            user_id: msg.from().map(|u| u.id.to_string())
                                .unwrap_or_default(),
                            platform: "telegram".to_string(),
                            chat_id: msg.chat.id.to_string(),
                            thread_id: msg.thread_id.map(|id| id.to_string()),
                            content: InputContent::Text(text.to_string()),
                            reply_fn: Arc::new(move |event| {
                                // Handled by deliver()
                            }),
                        }).await;
                    }
                    respond(())
                }
            ));

        Dispatcher::builder(self.bot.clone(), handler)
            .build()
            .dispatch()
            .await;

        Ok(())
    }

    async fn deliver(&self, event: &DeliveryEvent) -> Result<(), GatewayError> {
        match event {
            DeliveryEvent::FinalMessage(text) => {
                self.bot.send_message(self.home_chat_id, text)
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
            }
            DeliveryEvent::MediaFile { path, caption } => {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                match ext {
                    "png" | "jpg" | "jpeg" | "webp" => {
                        self.bot.send_photo(self.home_chat_id, InputFile::file(path))
                            .caption(caption.clone().unwrap_or_default())
                            .await?;
                    }
                    "mp4" => {
                        self.bot.send_video(self.home_chat_id, InputFile::file(path))
                            .await?;
                    }
                    "ogg" | "mp3" => {
                        self.bot.send_voice(self.home_chat_id, InputFile::file(path))
                            .await?;
                    }
                    _ => {
                        self.bot.send_document(self.home_chat_id, InputFile::file(path))
                            .await?;
                    }
                }
            }
            DeliveryEvent::ApprovalRequest { tool, description, id } => {
                let keyboard = InlineKeyboardMarkup::new(vec![vec![
                    InlineKeyboardButton::callback("✅ Approve", format!("approve:{id}")),
                    InlineKeyboardButton::callback("❌ Deny", format!("deny:{id}")),
                ]]);
                self.bot.send_message(
                    self.home_chat_id,
                    format!("⚠️ **{}** requires approval:\n\n{}", tool, description)
                )
                .reply_markup(keyboard)
                .await?;
            }
            _ => {}
        }
        Ok(())
    }
}
```

---

## 5. HTTP Gateway (axum)

```rust
pub struct HttpGateway {
    bind: SocketAddr,
    auth_token: Option<String>,
}

pub fn router(state: Arc<AppState>) -> axum::Router {
    axum::Router::new()
        .route("/chat", post(chat_handler))
        .route("/chat/stream", post(chat_stream_handler))
        .route("/health", get(health_handler))
        .route("/version", get(version_handler))
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(TimeoutLayer::new(Duration::from_secs(300)))
        )
        .with_state(state)
}

async fn chat_handler(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, StatusCode> {
    verify_auth(&app, &headers)?;

    let result = app.run_agent(req.message, req.session_id).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(ChatResponse {
        response: result.final_response,
        session_id: result.session_id,
    }))
}

async fn chat_stream_handler(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ChatRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Returns Server-Sent Events for streaming
    let (tx, rx) = mpsc::channel::<AgentEvent>(64);

    tokio::spawn(async move {
        app.run_agent_with_events(req.message, req.session_id, tx).await.ok();
    });

    let stream = ReceiverStream::new(rx).map(|event| {
        let data = serde_json::to_string(&event).unwrap_or_default();
        Ok(Event::default().data(data))
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
```

---

## 6. Delivery Routing

Talon supports multiple simultaneous delivery targets per message:

```rust
pub enum DeliverTarget {
    Origin,                              // back to sender
    Local,                               // save to file, no delivery
    All,                                 // all connected home channels
    Specific { platform: String, chat_id: String, thread_id: Option<String> },
}

pub struct GatewayRouter {
    gateways: HashMap<String, Arc<dyn Gateway>>,
}

impl GatewayRouter {
    pub async fn deliver_to(
        &self,
        target: &DeliverTarget,
        origin: &AgentInput,
        event: DeliveryEvent,
    ) -> Result<(), GatewayError> {
        match target {
            DeliverTarget::Origin => {
                let gw = self.gateways.get(&origin.platform)
                    .ok_or(GatewayError::UnknownPlatform(origin.platform.clone()))?;
                gw.deliver(&event).await?;
            }
            DeliverTarget::All => {
                let futures: Vec<_> = self.gateways.values()
                    .map(|gw| gw.deliver(&event))
                    .collect();
                futures::future::join_all(futures).await;
            }
            DeliverTarget::Specific { platform, chat_id, thread_id } => {
                let gw = self.gateways.get(platform)
                    .ok_or(GatewayError::UnknownPlatform(platform.clone()))?;
                gw.deliver(&event).await?;
            }
            DeliverTarget::Local => {
                // Write to ~/.talon/data/cron/output/<timestamp>.md
                self.save_locally(&event).await?;
            }
        }
        Ok(())
    }
}
```
---

## Related Documents

### Depends On
- [Cargo Workspace Design](12_Workspace_And_Crate_Structure.md)

### Used By
- [Telegram Integration](../05_API_Bindings/45_Telegram_Integration.md)
- [Discord Integration](../05_API_Bindings/46_Discord_Integration.md)
- [Send Message Tool](../04_Core_Features/35a_Send_Message_Tool.md)

### See Also
- [Messaging Platform Gateway](../05_API_Bindings/44a_Messaging_Platform_Gateway.md)
- [Streaming SSE Parser](../05_API_Bindings/44_Streaming_SSE_Parser.md)
- [Config System](18a_Config_System.md)
- [Approval Membrane](17a_Approval_Membrane.md)

