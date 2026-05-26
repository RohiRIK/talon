# Telegram Integration

> **Status:** ✅ Complete
> **Category:** API Bindings

---

## 1. Crate: teloxide

`teloxide` is the standard Telegram Bot API client for Rust.
It provides:
- Typed Bot API wrapper
- Handler dispatching (dptree-based)
- Webhook + long-polling support
- File download/upload
- Inline keyboards

```toml
[dependencies]
teloxide = { version = "0.13", features = ["macros", "webhooks", "throttle"] }
```

---

## 2. Bot Initialization

```rust
pub struct TelegramGateway {
    bot: AutoSend<Throttle<Bot>>,
    allowed_user_ids: Vec<UserId>,
    home_chat_id: ChatId,
}

impl TelegramGateway {
    pub fn new(config: &TelegramConfig) -> Self {
        let bot = Bot::new(&config.bot_token)
            .throttle(Limits::default())  // Built-in rate limit handling
            .auto_send();

        Self {
            bot,
            allowed_user_ids: config.allowed_user_ids.iter()
                .map(|&id| UserId(id))
                .collect(),
            home_chat_id: ChatId(config.home_chat_id),
        }
    }

    fn is_allowed(&self, user: &User) -> bool {
        self.allowed_user_ids.is_empty()  // empty = allow all
            || self.allowed_user_ids.contains(&user.id)
    }
}
```

---

## 3. Long Polling Listener

```rust
async fn listen(&self, input_tx: mpsc::Sender<AgentInput>) {
    let bot = self.bot.clone();
    let allowed = self.allowed_user_ids.clone();
    let tx = input_tx.clone();

    let handler = dptree::entry()
        // Text messages
        .branch(
            Update::filter_message()
                .filter(|msg: Message| msg.text().is_some())
                .endpoint(move |bot: AutoSend<Throttle<Bot>>, msg: Message| {
                    let tx = tx.clone();
                    let allowed = allowed.clone();
                    async move {
                        let user = match msg.from() {
                            Some(u) if allowed.is_empty() || allowed.contains(&u.id) => u,
                            _ => return respond(()),
                        };

                        tx.send(AgentInput {
                            session_id: Uuid::new_v4(),
                            user_id: user.id.to_string(),
                            platform: "telegram".to_string(),
                            chat_id: msg.chat.id.to_string(),
                            thread_id: msg.thread_id.map(|id| id.to_string()),
                            content: InputContent::Text(msg.text().unwrap().to_string()),
                        }).await.ok();

                        respond(())
                    }
                })
        )
        // Callback queries (inline keyboard button presses)
        .branch(
            Update::filter_callback_query()
                .endpoint(move |_bot, query: CallbackQuery| async move {
                    if let Some(data) = &query.data {
                        if data.starts_with("approve:") || data.starts_with("deny:") {
                            // Handle approval responses
                        }
                    }
                    respond(())
                })
        );

    Dispatcher::builder(bot, handler)
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}
```

---

## 4. Message Delivery

```rust
async fn deliver(&self, event: &DeliveryEvent, chat_id: ChatId) -> Result<(), GatewayError> {
    match event {
        DeliveryEvent::FinalMessage(text) => {
            // Split if over 4096 chars
            for chunk in split_message(text, 4000) {
                let normalized = normalize_for_platform(&chunk, "telegram");
                self.bot
                    .send_message(chat_id, normalized)
                    .parse_mode(ParseMode::Html)
                    .await
                    .map_err(GatewayError::Telegram)?;
            }
        }
        DeliveryEvent::MediaFile { path, caption } => {
            self.deliver_media(chat_id, path, caption.as_deref()).await?;
        }
        DeliveryEvent::ApprovalRequest { tool, description, id } => {
            let keyboard = InlineKeyboardMarkup::new(vec![vec![
                InlineKeyboardButton::callback("✅ Approve", format!("approve:{id}")),
                InlineKeyboardButton::callback("❌ Deny", format!("deny:{id}")),
            ]]);
            self.bot
                .send_message(chat_id, format!(
                    "⚠️ <b>{tool}</b> requires approval:\n\n<code>{description}</code>"
                ))
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        _ => {}
    }
    Ok(())
}

async fn deliver_media(
    &self,
    chat_id: ChatId,
    path: &Path,
    caption: Option<&str>,
) -> Result<(), GatewayError> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("bin");
    let file = InputFile::file(path);

    match ext {
        "png" | "jpg" | "jpeg" | "webp" | "gif" => {
            let mut req = self.bot.send_photo(chat_id, file);
            if let Some(cap) = caption { req = req.caption(cap); }
            req.await?;
        }
        "mp4" | "mov" => {
            self.bot.send_video(chat_id, file).await?;
        }
        "ogg" | "mp3" | "wav" => {
            self.bot.send_voice(chat_id, file).await?;
        }
        _ => {
            let mut req = self.bot.send_document(chat_id, file);
            if let Some(cap) = caption { req = req.caption(cap); }
            req.await?;
        }
    }
    Ok(())
}
```

---

## 5. Webhook Mode

For production deployments (better than long-polling):

```rust
pub async fn run_with_webhook(bot: Bot, url: &str, port: u16) {
    let webhook_url = reqwest::Url::parse(url).unwrap();

    Dispatcher::builder(bot.clone(), handler)
        .build()
        .dispatch_with_listener(
            webhooks::axum(bot, webhooks::Options::new(webhook_url))
                .await
                .unwrap(),
            LoggingErrorHandler::with_custom_text("Webhook error"),
        )
        .await;
}
```

```toml
[gateway.telegram]
bot_token = "${env:TELEGRAM_BOT_TOKEN}"
home_chat_id = 123456789
allowed_user_ids = [123456789]
mode = "webhook"     # or "polling"
webhook_url = "https://myserver.com/telegram/webhook"
webhook_port = 8443
```
---

## Related Documents

### Depends On
- [Gateway Architecture](../02_Architecture/18_Gateway_MultiChannel_Architecture.md)

### See Also
- [Discord Integration](46_Discord_Integration.md)
- [Send Message Tool](../04_Core_Features/35a_Send_Message_Tool.md)
- [Voice Mode](../04_Core_Features/37a_Voice_Mode.md)

