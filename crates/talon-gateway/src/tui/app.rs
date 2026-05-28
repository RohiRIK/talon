use tokio::sync::oneshot;

/// A message in the chat history.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
    System,
}

/// An active or completed tool call shown in the ToolPanel.
#[derive(Debug, Clone)]
pub struct ActiveTool {
    pub id: String,
    pub name: String,
    pub state: ToolState,
    pub output: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolState {
    Running,
    Done,
    Error,
}

/// Adaptive layout modes based on terminal width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// <80 cols — ChatView stacked above InputBar; ToolPanel hidden.
    Stacked,
    /// ≥120 cols — ChatView + ToolPanel side-by-side.
    SideBySide,
}

impl LayoutMode {
    pub fn from_width(cols: u16) -> Self {
        if cols >= 120 {
            Self::SideBySide
        } else {
            Self::Stacked
        }
    }
}

/// Status information shown in the StatusBar.
#[derive(Debug, Clone)]
pub struct StatusInfo {
    pub model: String,
    pub session_id: String,
    pub token_count: Option<usize>,
    pub native_sandbox: bool,
    pub thinking: bool,
}

impl Default for StatusInfo {
    fn default() -> Self {
        Self {
            model: String::from("unknown"),
            session_id: String::new(),
            token_count: None,
            native_sandbox: false,
            thinking: false,
        }
    }
}

/// All messages dispatched to the MVU update function.
#[derive(Debug)]
pub enum Msg {
    UserSubmit(String),
    AssistantText(String),
    ToolStarted { id: String, name: String },
    ToolDone { id: String, output: String },
    ToolError { id: String, error: String },
    ApprovalRequested {
        call_id: String,
        tool_name: String,
        args: serde_json::Value,
        tx: oneshot::Sender<bool>,
    },
    AgentDone,
    AgentFailed(String),
    Resize { cols: u16, rows: u16 },
    ToggleToolPanel,
    ScrollUp,
    ScrollDown,
    Quit,
}

/// The MVU model — all UI state.
///
/// `update(Msg) -> App` is a pure transformation; no mutation.
pub struct App {
    pub messages: Vec<ChatMessage>,
    pub tool_calls: Vec<ActiveTool>,
    pub layout_mode: LayoutMode,
    pub status: StatusInfo,
    pub scroll_offset: usize,
    pub tool_panel_visible: bool,
    pub pending_approval: Option<PendingApproval>,
    pub should_quit: bool,
}

/// An approval request waiting for user input.
#[derive(Debug)]
pub struct PendingApproval {
    pub call_id: String,
    pub tool_name: String,
    pub args: serde_json::Value,
    pub tx: oneshot::Sender<bool>,
}

impl App {
    pub fn new(session_id: impl Into<String>, model: impl Into<String>) -> Self {
        let status = StatusInfo {
            session_id: session_id.into(),
            model: model.into(),
            ..Default::default()
        };
        Self {
            messages: Vec::new(),
            tool_calls: Vec::new(),
            layout_mode: LayoutMode::Stacked,
            status,
            scroll_offset: 0,
            tool_panel_visible: false,
            pending_approval: None,
            should_quit: false,
        }
    }

    /// Pure update: consume self, return new state.
    pub fn update(mut self, msg: Msg) -> Self {
        match msg {
            Msg::UserSubmit(text) => {
                self.messages.push(ChatMessage {
                    role: ChatRole::User,
                    content: text,
                });
                self.status.thinking = true;
                self.scroll_to_bottom();
            }
            Msg::AssistantText(text) => {
                self.messages.push(ChatMessage {
                    role: ChatRole::Assistant,
                    content: text,
                });
                self.status.thinking = false;
                self.scroll_to_bottom();
            }
            Msg::ToolStarted { id, name } => {
                self.tool_calls.push(ActiveTool {
                    id,
                    name,
                    state: ToolState::Running,
                    output: None,
                });
                self.tool_panel_visible = true;
            }
            Msg::ToolDone { id, output } => {
                if let Some(t) = self.tool_calls.iter_mut().find(|t| t.id == id) {
                    t.state = ToolState::Done;
                    t.output = Some(output);
                }
            }
            Msg::ToolError { id, error } => {
                if let Some(t) = self.tool_calls.iter_mut().find(|t| t.id == id) {
                    t.state = ToolState::Error;
                    t.output = Some(error);
                }
            }
            Msg::ApprovalRequested {
                call_id,
                tool_name,
                args,
                tx,
            } => {
                self.pending_approval = Some(PendingApproval {
                    call_id,
                    tool_name,
                    args,
                    tx,
                });
            }
            Msg::AgentDone => {
                self.status.thinking = false;
            }
            Msg::AgentFailed(err) => {
                self.status.thinking = false;
                self.messages.push(ChatMessage {
                    role: ChatRole::System,
                    content: format!("[error] {err}"),
                });
            }
            Msg::Resize { cols, rows: _ } => {
                self.layout_mode = LayoutMode::from_width(cols);
            }
            Msg::ToggleToolPanel => {
                self.tool_panel_visible = !self.tool_panel_visible;
            }
            Msg::ScrollUp => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
            }
            Msg::ScrollDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            Msg::Quit => {
                self.should_quit = true;
            }
        }
        self
    }

    fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_app() -> App {
        App::new("sess-1", "claude-opus-4-7")
    }

    #[test]
    fn user_submit_adds_message() {
        let app = make_app().update(Msg::UserSubmit("hi".into()));
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, ChatRole::User);
        assert_eq!(app.messages[0].content, "hi");
        assert!(app.status.thinking);
    }

    #[test]
    fn assistant_text_adds_message_and_clears_thinking() {
        let app = make_app()
            .update(Msg::UserSubmit("hi".into()))
            .update(Msg::AssistantText("hello".into()));
        assert_eq!(app.messages.len(), 2);
        assert_eq!(app.messages[1].role, ChatRole::Assistant);
        assert!(!app.status.thinking);
    }

    #[test]
    fn tool_started_shows_panel() {
        let app = make_app().update(Msg::ToolStarted {
            id: "t1".into(),
            name: "read_file".into(),
        });
        assert!(app.tool_panel_visible);
        assert_eq!(app.tool_calls[0].state, ToolState::Running);
    }

    #[test]
    fn tool_done_updates_state() {
        let app = make_app()
            .update(Msg::ToolStarted {
                id: "t1".into(),
                name: "read_file".into(),
            })
            .update(Msg::ToolDone {
                id: "t1".into(),
                output: "contents".into(),
            });
        assert_eq!(app.tool_calls[0].state, ToolState::Done);
        assert_eq!(app.tool_calls[0].output.as_deref(), Some("contents"));
    }

    #[test]
    fn toggle_tool_panel_flips_visibility() {
        let app = make_app();
        assert!(!app.tool_panel_visible);
        let app = app.update(Msg::ToggleToolPanel);
        assert!(app.tool_panel_visible);
        let app = app.update(Msg::ToggleToolPanel);
        assert!(!app.tool_panel_visible);
    }

    #[test]
    fn quit_sets_flag() {
        let app = make_app().update(Msg::Quit);
        assert!(app.should_quit);
    }

    #[test]
    fn resize_updates_layout_mode() {
        let app = make_app().update(Msg::Resize { cols: 120, rows: 40 });
        assert_eq!(app.layout_mode, LayoutMode::SideBySide);

        let app = make_app().update(Msg::Resize { cols: 79, rows: 40 });
        assert_eq!(app.layout_mode, LayoutMode::Stacked);
    }

    #[test]
    fn layout_mode_boundary() {
        assert_eq!(LayoutMode::from_width(119), LayoutMode::Stacked);
        assert_eq!(LayoutMode::from_width(120), LayoutMode::SideBySide);
        assert_eq!(LayoutMode::from_width(200), LayoutMode::SideBySide);
    }

    #[test]
    fn agent_failed_adds_system_message() {
        let app = make_app().update(Msg::AgentFailed("oops".into()));
        assert!(app.messages[0].content.contains("oops"));
        assert_eq!(app.messages[0].role, ChatRole::System);
        assert!(!app.status.thinking);
    }
}
