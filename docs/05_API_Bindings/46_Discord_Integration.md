# Discord Integration

> **Status:** ✅ Complete
> **Category:** API Bindings

---

## 1. Crate: serenity

`serenity` is the most mature Discord library for Rust:

```toml
[dependencies]
serenity = { version = "0.12", features = ["client", "gateway", "model", "cache"] }
```

---

## 2. Discord Gateway Implementation

```rust
pub struct DiscordGateway {
    token: String,
    home_channel_id: ChannelId,
    allowed_guilds: Vec<GuildId>,
    input_tx: Arc<mpsc::Sender<AgentInput>>,
}

struct Handler {
    input_tx: Arc<mpsc::Sender<AgentInput>>,
    bot_id: Arc<tokio::sync::OnceCell<UserId>>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        self.bot_id.set(ready.user.id).ok();
        tracing::info!("Discord: connected as {}", ready.user.name);
    }

    async fn message(&self, ctx: Context, msg: serenity::model::channel::Message) {
        // Ignore own messages
        if let Some(bot_id) = self.bot_id.get() {
            if msg.author.id == *bot_id { return; }
        }

        // Only respond to @mentions or DMs
        let is_dm = msg.guild_id.is_none();
        let is_mention = msg.mentions_me(&ctx).await.unwrap_or(false);

        if !is_dm && !is_mention { return; }

        // Strip @mention from message
        let content = if is_mention {
            msg.content_safe(&ctx).await
                .split_once('>').map(|(_, s)| s.trim()).unwrap_or(&msg.content)
                .to_string()
        } else {
            msg.content.clone()
        };

        let _ = self.input_tx.send(AgentInput {
            session_id: Uuid::new_v4(),
            user_id: msg.author.id.to_string(),
            platform: "discord".to_string(),
            chat_id: msg.channel_id.to_string(),
            thread_id: None,
            content: InputContent::Text(content),
        }).await;
    }
}

#[async_trait]
impl Gateway for DiscordGateway {
    fn name(&self) -> &str { "discord" }

    async fn listen(&self, tx: mpsc::Sender<AgentInput>) -> Result<(), GatewayError> {
        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT;

        let mut client = Client::builder(&self.token, intents)
            .event_handler(Handler { input_tx: Arc::new(tx), bot_id: Default::default() })
            .await
            .map_err(|e| GatewayError::Discord(e.to_string()))?;

        client.start().await
            .map_err(|e| GatewayError::Discord(e.to_string()))
    }

    async fn deliver(&self, event: &DeliveryEvent) -> Result<(), GatewayError> {
        // Delivery is handled via HTTP API call (serenity's Http client)
        // Implementation follows same pattern as Telegram
        Ok(())
    }
}
```

---

## 3. Slash Commands (optional)

```rust
// Register /ask slash command
async fn register_commands(ctx: &Context) {
    Command::create_global_command(&ctx.http, |cmd| {
        cmd.name("ask")
            .description("Ask Talon a question")
            .create_option(|opt| {
                opt.name("message")
                    .description("Your message")
                    .kind(CommandOptionType::String)
                    .required(true)
            })
    }).await.ok();
}
```

---

## 4. Configuration

```toml
[gateway.discord]
enabled = false
bot_token = "${env:DISCORD_BOT_TOKEN}"
home_channel_id = 1234567890
allowed_guilds = []    # empty = all guilds
```
---

## Related Documents

### Depends On
- [Gateway Architecture](../02_Architecture/18_Gateway_MultiChannel_Architecture.md)

### See Also
- [Telegram Integration](45_Telegram_Integration.md)
- [Send Message Tool](../04_Core_Features/35a_Send_Message_Tool.md)

