#![cfg(feature = "telegram")]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use teloxide::{
    Bot,
    dispatching::UpdateFilterExt,
    prelude::*,
    types::{Message as TgMessage, Update},
};
use tokio::sync::mpsc;

use talon_core::events::AgentEvent;

use crate::{Gateway, GatewayContext, GatewayError, RenderMode, normalize::normalize_markdown};

/// Telegram gateway using long-polling.
///
/// Requires `TELEGRAM_BOT_TOKEN` environment variable.
/// Each message starts a fresh agent session (single-turn per message).
/// For multi-turn within one Telegram conversation, use `session_id = chat_id`.
pub struct TelegramGateway {
    ctx: Arc<GatewayContext>,
    token: String,
}

impl TelegramGateway {
    /// Create a new gateway. Returns an error if TELEGRAM_BOT_TOKEN is not set.
    pub fn from_env(ctx: Arc<GatewayContext>) -> Result<Self, GatewayError> {
        let token = std::env::var("TELEGRAM_BOT_TOKEN")
            .map_err(|_| GatewayError::Config("TELEGRAM_BOT_TOKEN not set".to_string()))?;
        Ok(Self { ctx, token })
    }

    pub fn new(ctx: Arc<GatewayContext>, token: impl Into<String>) -> Self {
        Self {
            ctx,
            token: token.into(),
        }
    }
}

impl Gateway for TelegramGateway {
    fn name(&self) -> &str {
        "telegram"
    }

    fn render_mode(&self) -> RenderMode {
        // Telegram supports limited markdown — use Accessible to strip unsupported syntax.
        RenderMode::Accessible
    }

    fn run(&self) -> Pin<Box<dyn Future<Output = Result<(), GatewayError>> + Send + '_>> {
        Box::pin(async move {
            let bot = Bot::new(&self.token);
            let ctx = Arc::clone(&self.ctx);

            tracing::info!("Telegram gateway starting (long-polling)…");

            let handler = Update::filter_message()
                .filter_map(|msg: TgMessage| async move {
                    msg.text().map(|t| (msg.chat.id, t.to_string()))
                })
                .endpoint(move |bot: Bot, (chat_id, text): (ChatId, String)| {
                    let ctx = Arc::clone(&ctx);
                    async move {
                        let response = run_telegram_turn(ctx, chat_id, text).await;
                        bot.send_message(chat_id, response)
                            .await
                            .map_err(|e| {
                                tracing::warn!("failed to send Telegram reply: {e}");
                            })
                            .ok();
                        respond(())
                    }
                });

            Dispatcher::builder(bot, handler)
                .enable_ctrlc_handler()
                .build()
                .dispatch()
                .await;

            Ok(())
        })
    }
}

async fn run_telegram_turn(
    ctx: Arc<GatewayContext>,
    chat_id: ChatId,
    text: String,
) -> String {
    let session_id = format!("tg-{}", chat_id);
    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(64);

    let collector = tokio::spawn(async move {
        let mut last_text = String::new();
        while let Some(event) = event_rx.recv().await {
            match event {
                AgentEvent::Text { content } => last_text = content,
                AgentEvent::ApprovalRequested { tx, .. } => {
                    // Telegram auto-denies Dangerous tools for security.
                    tx.send(false).ok();
                }
                AgentEvent::Completed | AgentEvent::Failed(_) => break,
                _ => {}
            }
        }
        last_text
    });

    let mut agent = ctx.build_agent(event_tx);
    if let Err(e) = agent.run(&session_id, text).await {
        tracing::warn!("agent error in Telegram turn: {e}");
        return format!("Sorry, I encountered an error: {e}");
    }

    let raw = collector.await.unwrap_or_default();
    // Telegram supports a subset of markdown; normalize to Accessible mode.
    normalize_markdown(&raw, RenderMode::Accessible)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use talon_llm::MockProvider;

    fn make_ctx() -> Arc<GatewayContext> {
        Arc::new(GatewayContext::new(Arc::new(MockProvider::text(
            "telegram reply",
            "end_turn",
        ))))
    }

    #[test]
    fn telegram_gateway_name() {
        let gw = TelegramGateway::new(make_ctx(), "fake-token");
        assert_eq!(gw.name(), "telegram");
    }

    #[test]
    fn telegram_from_env_errors_without_token() {
        unsafe { std::env::remove_var("TELEGRAM_BOT_TOKEN") };
        let result = TelegramGateway::from_env(make_ctx());
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn run_telegram_turn_returns_text() {
        let ctx = make_ctx();
        let chat_id = ChatId(42);
        let reply = run_telegram_turn(ctx, chat_id, "hello".to_string()).await;
        assert!(!reply.is_empty());
    }
}
