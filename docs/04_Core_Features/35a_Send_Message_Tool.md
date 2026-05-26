# Send Message Tool & Gateway Dispatch

> **Status:** ✅ Complete
> **Category:** Core Features
> **Last corrected:** dogfood pass 3

---

## 1. Overview

`send_message` is Talon's outbound delivery tool. It routes messages to connected
communication platforms — Telegram, Discord, Slack, or local stdout.

The tool sits at the intersection of the agent loop and the gateway layer:
the agent calls it as a tool; the gateway layer handles actual delivery.

---

## 2. Tool Implementation

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendMessageParams {
    /// Message text (supports platform markdown)
    pub message: String,
    /// Target: "telegram", "discord:#channel", "telegram:-1001234:567", etc.
    /// Defaults to the origin gateway (where the current session came from)
    pub target: Option<String>,
}

pub struct SendMessageTool {
    router: Arc<GatewayRouter>,
}

#[async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &str { "send_message" }

    fn description(&self) -> &str {
        "Send a message to a connected platform (Telegram, Discord, Slack, etc.). \
         Use action='list' target to discover available channels before sending \
         to a specific channel."
    }

    fn approval_level(&self) -> ApprovalLevel { ApprovalLevel::Confirmation }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let p: SendMessageParams = serde_json::from_value(args)?;

        // Handle list action
        if p.message.trim() == "list" || p.target.as_deref() == Some("list") {
            let channels = self.router.list_channels().await;
            return Ok(ToolResult::text(format_channel_list(&channels)));
        }

        let target = resolve_target(p.target.as_deref(), ctx)?;

        self.router.send(DeliveryRequest {
            target,
            message: p.message.clone(),
            media: extract_media_paths(&p.message),
            session_id: ctx.session_id,
        }).await.map_err(ToolError::Gateway)?;

        Ok(ToolResult::text(format!("Message sent to {}", target.display())))
    }
}
```

---

## 3. GatewayRouter

The router maintains a registry of connected gateways and resolves delivery targets.

```rust
pub struct GatewayRouter {
    gateways: RwLock<HashMap<String, Arc<dyn Gateway>>>,
    home_channels: RwLock<HashMap<String, DeliverTarget>>,
}

impl GatewayRouter {
    /// Send to a specific target
    pub async fn send(&self, req: DeliveryRequest) -> Result<(), GatewayError> {
        match &req.target {
            DeliverTarget::Origin => {
                // Send back to the gateway that initiated the session
                let gw = self.get_origin_gateway(&req.session_id).await?;
                gw.send(req.message, req.media).await
            }
            DeliverTarget::All => {
                // Fan out to all home channels
                let gws = self.gateways.read().await;
                let futures: Vec<_> = gws.values()
                    .map(|gw| gw.send_to_home(req.message.clone(), req.media.clone()))
                    .collect();
                let results = futures::future::join_all(futures).await;
                // Collect errors but don't fail on partial delivery
                for r in results {
                    if let Err(e) = r {
                        tracing::warn!("Partial delivery failure: {e}");
                    }
                }
                Ok(())
            }
            DeliverTarget::Platform { platform, chat_id, thread_id } => {
                let gws = self.gateways.read().await;
                let gw = gws.get(platform.as_str())
                    .ok_or_else(|| GatewayError::UnknownPlatform(platform.clone()))?;
                gw.send_to(chat_id, thread_id.as_deref(), req.message, req.media).await
            }
            DeliverTarget::Local => {
                // Write to ~/.talon/cron/output/<session_id>.txt
                self.write_local_output(&req).await
            }
        }
    }

    pub async fn list_channels(&self) -> Vec<ChannelInfo> {
        let gws = self.gateways.read().await;
        let mut channels = vec![];

        for (name, gw) in gws.iter() {
            if let Ok(ch) = gw.list_channels().await {
                channels.extend(ch.into_iter().map(|c| ChannelInfo {
                    platform: name.clone(),
                    ..c
                }));
            }
        }
        channels
    }
}
```

---

## 4. Gateway Trait

```rust
#[async_trait]
pub trait Gateway: Send + Sync {
    fn platform(&self) -> &str;

    /// Send to home channel
    async fn send_to_home(
        &self,
        message: String,
        media: Vec<MediaAttachment>,
    ) -> Result<(), GatewayError>;

    /// Send to specific chat/thread
    async fn send_to(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        message: String,
        media: Vec<MediaAttachment>,
    ) -> Result<(), GatewayError>;

    /// List available channels/groups this gateway can reach
    async fn list_channels(&self) -> Result<Vec<ChannelInfo>, GatewayError> {
        Ok(vec![]) // default: no enumeration
    }

    /// Receive incoming messages (returns stream of inbound events)
    async fn receive(&self) -> Result<BoxStream<'static, InboundMessage>, GatewayError>;
}
```

---

## 5. Target Resolution

```rust
fn resolve_target(
    raw: Option<&str>,
    ctx: &ToolContext,
) -> Result<DeliverTarget, ToolError> {
    match raw {
        None | Some("origin") => Ok(DeliverTarget::Origin),
        Some("all") => Ok(DeliverTarget::All),
        Some("local") => Ok(DeliverTarget::Local),
        Some(s) => {
            // Format: "platform:chat_id" or "platform:chat_id:thread_id"
            // e.g. "telegram:-1001234567890:17585"
            //      "discord:#engineering"
            let parts: Vec<&str> = s.splitn(3, ':').collect();
            match parts.as_slice() {
                [platform, chat_id] => Ok(DeliverTarget::Platform {
                    platform: platform.to_string(),
                    chat_id: chat_id.to_string(),
                    thread_id: None,
                }),
                [platform, chat_id, thread_id] => Ok(DeliverTarget::Platform {
                    platform: platform.to_string(),
                    chat_id: chat_id.to_string(),
                    thread_id: Some(thread_id.to_string()),
                }),
                _ => Err(ToolError::InvalidParams(
                    format!("Invalid target format: '{s}'")
                )),
            }
        }
    }
}
```

---

## 6. Media Extraction

The `message` field supports a `MEDIA:/path/to/file` embedded marker
(identical to Hermes convention) that gets extracted before sending:

```rust
fn extract_media_paths(message: &str) -> (String, Vec<MediaAttachment>) {
    let media_re = regex::Regex::new(r"MEDIA:([^\s]+)").unwrap();

    let mut attachments = vec![];
    let clean_message = media_re.replace_all(message, |caps: &regex::Captures| {
        let path = PathBuf::from(&caps[1]);
        let attachment = match path.extension().and_then(|e| e.to_str()) {
            Some("png" | "jpg" | "jpeg" | "webp") => MediaAttachment::Image(path),
            Some("ogg" | "mp3" | "wav") => MediaAttachment::Audio(path),
            Some("mp4" | "webm") => MediaAttachment::Video(path),
            _ => MediaAttachment::File(path),
        };
        attachments.push(attachment);
        ""
    });

    (clean_message.trim().to_string(), attachments)
}
```

---

## 7. Telegram Gateway Implementation

```rust
pub struct TelegramGateway {
    bot: teloxide::Bot,
    home_chat_id: ChatId,
    home_thread_id: Option<ThreadId>,
}

#[async_trait]
impl Gateway for TelegramGateway {
    fn platform(&self) -> &str { "telegram" }

    async fn send_to_home(
        &self,
        message: String,
        media: Vec<MediaAttachment>,
    ) -> Result<(), GatewayError> {
        self.send_to(
            &self.home_chat_id.to_string(),
            self.home_thread_id.map(|t| t.to_string()).as_deref(),
            message,
            media,
        ).await
    }

    async fn send_to(
        &self,
        chat_id: &str,
        thread_id: Option<&str>,
        message: String,
        media: Vec<MediaAttachment>,
    ) -> Result<(), GatewayError> {
        let cid: ChatId = chat_id.parse().map_err(|_| GatewayError::InvalidChatId)?;

        // Split long messages (Telegram limit: 4096 chars)
        for chunk in split_message(&message, 4096) {
            let mut req = self.bot.send_message(cid, chunk)
                .parse_mode(teloxide::types::ParseMode::MarkdownV2);

            if let Some(tid) = thread_id.and_then(|t| t.parse::<i32>().ok()) {
                req = req.message_thread_id(tid);
            }

            req.await.map_err(GatewayError::Telegram)?;
        }

        // Send media attachments
        for attachment in media {
            match attachment {
                MediaAttachment::Image(path) => {
                    self.bot.send_photo(cid, teloxide::types::InputFile::file(path))
                        .await.map_err(GatewayError::Telegram)?;
                }
                MediaAttachment::Audio(path) => {
                    self.bot.send_voice(cid, teloxide::types::InputFile::file(path))
                        .await.map_err(GatewayError::Telegram)?;
                }
                _ => { /* handle other types */ }
            }
        }

        Ok(())
    }
}
```

---

## 8. Message Formatting Per Platform

Different platforms render markdown differently — Talon applies per-platform transforms:

```rust
pub fn format_for_platform(raw: &str, platform: &str) -> String {
    match platform {
        "telegram" => telegram_escape_md(raw),
        "discord"  => discord_format(raw),
        "slack"    => slack_format(raw),
        "cli"      => ansi_format(raw),
        _          => raw.to_string(),
    }
}

fn telegram_escape_md(s: &str) -> String {
    // Telegram MarkdownV2 requires escaping these chars outside of markdown syntax:
    // . ! ( ) [ ] { } + - = # | > ~ `
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '.' | '!' | '(' | ')' | '[' | ']' | '{' | '}' | '+'
            | '-' | '=' | '#' | '|' | '>' | '~' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}
```

> **Last corrected:** dogfood pass 4
---

## Related Documents

### Depends On
- [Gateway Architecture](../02_Architecture/18_Gateway_MultiChannel_Architecture.md)
- [Tool System Architecture](../02_Architecture/16_Tool_System_Architecture.md)

### See Also
- [Telegram Integration](../05_API_Bindings/45_Telegram_Integration.md)
- [Discord Integration](../05_API_Bindings/46_Discord_Integration.md)

