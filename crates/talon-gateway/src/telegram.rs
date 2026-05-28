#![cfg(feature = "telegram")]

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use teloxide::{
    Bot,
    dispatching::UpdateFilterExt,
    prelude::*,
    types::{Message as TgMessage, Update},
};
use tokio::sync::{RwLock, mpsc};

use talon_core::events::AgentEvent;

use crate::{Gateway, GatewayContext, GatewayError, RenderMode, normalize::normalize_markdown};

// ── User auth ─────────────────────────────────────────────────────────────────

/// Outcome of an auth check against the whitelist.
#[derive(Debug, PartialEq, Eq)]
enum AuthResult {
    /// No owner registered yet — caller should register this user.
    Unregistered,
    /// User is on the whitelist.
    Allowed,
    /// User is not on the whitelist.
    Denied,
}

/// Whitelist of Telegram user IDs permitted to interact with this bot.
///
/// ## First-run (empty list)
/// The first user to send any message is auto-registered as the owner.
/// Their ID is written to `~/.talon/telegram_owner` so it survives restarts.
///
/// ## Explicit override
/// Set `TELEGRAM_ALLOWED_USER_IDS=123456789,987654321` (comma-separated) to
/// bypass auto-registration. Env var always takes priority over the owner file.
struct UserAuth {
    allowed: Arc<RwLock<Vec<u64>>>,
    owner_file: PathBuf,
}

impl UserAuth {
    fn load() -> Self {
        let owner_file = talon_home().join("telegram_owner");

        // Priority 1: explicit env var (comma-separated user IDs)
        let mut allowed: Vec<u64> = std::env::var("TELEGRAM_ALLOWED_USER_IDS")
            .unwrap_or_default()
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        // Priority 2: persisted first-run registration
        if allowed.is_empty()
            && let Ok(content) = std::fs::read_to_string(&owner_file)
            && let Ok(id) = content.trim().parse::<u64>()
        {
            allowed.push(id);
        }

        Self {
            allowed: Arc::new(RwLock::new(allowed)),
            owner_file,
        }
    }

    async fn check(&self, user_id: u64) -> AuthResult {
        let list = self.allowed.read().await;
        if list.is_empty() {
            AuthResult::Unregistered
        } else if list.contains(&user_id) {
            AuthResult::Allowed
        } else {
            AuthResult::Denied
        }
    }

    /// Register the first user as owner. No-op if an owner already exists.
    async fn register_first(&self, user_id: u64) {
        let mut list = self.allowed.write().await;
        if list.is_empty() {
            list.push(user_id);
            // Persist across restarts — silent failure is acceptable here.
            let _ = std::fs::write(&self.owner_file, user_id.to_string());
            tracing::info!("Telegram owner registered: user_id={user_id}");
        }
    }
}

fn talon_home() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".talon")
}

// ── Gateway ───────────────────────────────────────────────────────────────────

/// Telegram gateway using long-polling.
///
/// Requires `TELEGRAM_BOT_TOKEN` environment variable.
/// Each message starts a fresh agent session keyed by chat ID.
///
/// ## Auth
/// See [`UserAuth`] — first sender auto-registers; all others are rejected.
/// Override with `TELEGRAM_ALLOWED_USER_IDS=<id1,id2>`.
pub struct TelegramGateway {
    ctx: Arc<GatewayContext>,
    token: String,
}

impl TelegramGateway {
    /// Create a new gateway. Returns an error if `TELEGRAM_BOT_TOKEN` is not set.
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
            let auth = Arc::new(UserAuth::load());

            tracing::info!("Telegram gateway starting (long-polling)…");

            let handler = Update::filter_message().endpoint(
                move |bot: Bot, msg: TgMessage| {
                    let ctx = Arc::clone(&ctx);
                    let auth = Arc::clone(&auth);
                    async move {
                        // Non-text messages (stickers, photos, etc.) are silently ignored.
                        let text = match msg.text() {
                            Some(t) => t.to_string(),
                            None => return respond(()),
                        };
                        let user_id = msg.from.as_ref().map(|u| u.id.0).unwrap_or(0);
                        let chat_id = msg.chat.id;

                        match auth.check(user_id).await {
                            AuthResult::Unregistered => {
                                auth.register_first(user_id).await;
                                bot.send_message(
                                    chat_id,
                                    format!(
                                        "Welcome! Your Telegram ID `{user_id}` is now \
                                         registered as the bot owner. Only you can use \
                                         this assistant."
                                    ),
                                )
                                .await
                                .map_err(|e| tracing::warn!("send error: {e}"))
                                .ok();
                                let response = run_telegram_turn(ctx, chat_id, text).await;
                                bot.send_message(chat_id, response)
                                    .await
                                    .map_err(|e| tracing::warn!("send error: {e}"))
                                    .ok();
                            }
                            AuthResult::Allowed => {
                                let response = run_telegram_turn(ctx, chat_id, text).await;
                                bot.send_message(chat_id, response)
                                    .await
                                    .map_err(|e| tracing::warn!("send error: {e}"))
                                    .ok();
                            }
                            AuthResult::Denied => {
                                tracing::warn!(
                                    "Rejected unauthorized Telegram user {user_id}"
                                );
                                bot.send_message(chat_id, "This is a private assistant.")
                                    .await
                                    .map_err(|e| tracing::warn!("send error: {e}"))
                                    .ok();
                            }
                        }
                        respond(())
                    }
                },
            );

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
    normalize_markdown(&raw, RenderMode::Accessible)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use talon_llm::MockProvider;

    fn make_ctx() -> Arc<GatewayContext> {
        Arc::new(GatewayContext::new(Arc::new(MockProvider::text(
            "telegram reply",
            "end_turn",
        ))))
    }

    fn temp_auth() -> UserAuth {
        let tmp = std::env::temp_dir().join(format!(
            "talon_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        ));
        UserAuth {
            allowed: Arc::new(RwLock::new(vec![])),
            owner_file: tmp,
        }
    }

    // ── Gateway trait ──────────────────────────────────────────────────────────

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

    // ── UserAuth ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn auth_unregistered_when_empty() {
        let auth = temp_auth();
        assert_eq!(auth.check(12345).await, AuthResult::Unregistered);
    }

    #[tokio::test]
    async fn auth_allowed_after_registration() {
        let auth = temp_auth();
        auth.register_first(12345).await;
        assert_eq!(auth.check(12345).await, AuthResult::Allowed);
    }

    #[tokio::test]
    async fn auth_denied_for_other_user() {
        let auth = temp_auth();
        auth.register_first(12345).await;
        assert_eq!(auth.check(99999).await, AuthResult::Denied);
    }

    #[tokio::test]
    async fn auth_register_first_is_idempotent() {
        let auth = temp_auth();
        auth.register_first(12345).await;
        auth.register_first(99999).await; // second call is a no-op
        assert_eq!(auth.check(12345).await, AuthResult::Allowed);
        assert_eq!(auth.check(99999).await, AuthResult::Denied);
    }

    #[tokio::test]
    async fn auth_persists_owner_to_file() {
        let tmp = std::env::temp_dir().join(format!(
            "talon_persist_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        ));

        let auth = UserAuth {
            allowed: Arc::new(RwLock::new(vec![])),
            owner_file: tmp.clone(),
        };
        auth.register_first(77777).await;

        // Simulate restart: load from the persisted file.
        let content = std::fs::read_to_string(&tmp).expect("owner file written");
        assert_eq!(content.trim(), "77777");

        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn auth_env_var_takes_priority() {
        // Temporarily set the env var, then unset it after the test.
        unsafe { std::env::set_var("TELEGRAM_ALLOWED_USER_IDS", "55555,66666") };
        let auth = UserAuth::load();
        unsafe { std::env::remove_var("TELEGRAM_ALLOWED_USER_IDS") };

        assert_eq!(auth.check(55555).await, AuthResult::Allowed);
        assert_eq!(auth.check(66666).await, AuthResult::Allowed);
        assert_eq!(auth.check(99999).await, AuthResult::Denied);
    }
}
