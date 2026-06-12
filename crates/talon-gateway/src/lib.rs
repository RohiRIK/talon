use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;

use talon_core::agent::Agent;
use talon_core::events::AgentEvent;
use talon_core::tools::Tool;
use talon_core::tools::dispatcher::ToolDispatcher;
use talon_llm::LlmProvider;
use talon_memory::Database;

pub mod cli;
pub mod http;
pub mod normalize;
pub mod registry;
pub mod tui;
pub mod web;

#[cfg(feature = "telegram")]
pub mod telegram;

pub use registry::GatewayRegistry;

// ── Normalized message ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    User,
    Assistant,
    System,
}

/// Platform-normalized message — the common currency between all gateways.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub session_id: String,
    pub channel_id: Option<String>,
}

impl Message {
    pub fn user(content: impl Into<String>, session_id: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            session_id: session_id.into(),
            channel_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>, session_id: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            session_id: session_id.into(),
            channel_id: None,
        }
    }

    pub fn with_channel(mut self, channel_id: impl Into<String>) -> Self {
        self.channel_id = Some(channel_id.into());
        self
    }
}

// ── Render modes ──────────────────────────────────────────────────────────────

/// How the gateway renders output to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderMode {
    /// Raw text, no colour — works in CI and piped contexts.
    #[default]
    Plain,
    /// Line-by-line output, no escape sequences — screen readers, `--accessible`.
    Accessible,
    /// Full ratatui TUI with streaming markdown, diff view, inline images.
    Tui,
}

// ── Gateway errors ────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("agent error: {0}")]
    Agent(String),
    #[error("channel closed")]
    ChannelClosed,
    #[error("configuration error: {0}")]
    Config(String),
    #[error("network error: {0}")]
    Network(String),
}

// ── Shared agent-building context ────────────────────────────────────────────

/// Shared infrastructure injected into every gateway at construction time.
///
/// Each gateway call creates a fresh `ToolDispatcher` and `Agent` from these
/// components — tools are `Arc<dyn Tool>` so cloning is cheap.
pub struct GatewayContext {
    pub provider: Arc<dyn LlmProvider>,
    pub tools: Vec<Arc<dyn Tool>>,
    pub db: Option<Arc<Database>>,
}

impl GatewayContext {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            provider,
            tools: Vec::new(),
            db: None,
        }
    }

    pub fn with_tool(mut self, tool: Arc<dyn Tool>) -> Self {
        self.tools.push(tool);
        self
    }

    pub fn with_db(mut self, db: Arc<Database>) -> Self {
        self.db = Some(db);
        self
    }

    /// Build a fresh agent wired to the given event sender.
    pub fn build_agent(&self, event_tx: mpsc::Sender<AgentEvent>) -> Agent {
        let mut dispatcher = ToolDispatcher::new();
        for tool in &self.tools {
            dispatcher.register(Arc::clone(tool));
        }
        let agent = Agent::new(Arc::clone(&self.provider), dispatcher, event_tx);
        match &self.db {
            Some(db) => agent.with_db(Arc::clone(db)),
            None => agent,
        }
    }
}

// ── Gateway trait ─────────────────────────────────────────────────────────────

/// All gateways implement this trait.
///
/// `run()` returns `Pin<Box<dyn Future>>` (not `async fn`) so the trait is
/// object-safe and can be stored as `Arc<dyn Gateway>` in the registry.
pub trait Gateway: Send + Sync {
    fn name(&self) -> &str;
    fn render_mode(&self) -> RenderMode;
    fn run(&self) -> Pin<Box<dyn Future<Output = Result<(), GatewayError>> + Send + '_>>;
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn message_user_role() {
        let m = Message::user("hello", "s1");
        assert_eq!(m.role, Role::User);
        assert_eq!(m.content, "hello");
        assert_eq!(m.session_id, "s1");
        assert!(m.channel_id.is_none());
    }

    #[test]
    fn message_with_channel() {
        let m = Message::assistant("hi", "s1").with_channel("telegram");
        assert_eq!(m.channel_id.as_deref(), Some("telegram"));
    }

    #[test]
    fn render_mode_default_is_plain() {
        assert_eq!(RenderMode::default(), RenderMode::Plain);
    }

    #[test]
    fn message_serializes() {
        let m = Message::user("test", "sess");
        let json = serde_json::to_string(&m).expect("serialize");
        assert!(json.contains("test"));
        assert!(json.contains("sess"));
    }
}
