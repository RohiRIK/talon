pub mod app;
pub mod components;
pub mod layout;
pub mod render;

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

use talon_core::events::AgentEvent;

use crate::{Gateway, GatewayContext, GatewayError, RenderMode};
use app::{App, Msg};
use components::{
    chat::ChatView,
    input::{Input, InputBar},
    status::StatusBar,
    tools::ToolPanel,
};
use layout::SplitPane;
use render::detect_capabilities;

/// Full ratatui TUI gateway — MVU pattern (Elm-style).
///
/// Gateways that don't support a real terminal automatically degrade:
/// detect_capabilities() returns Plain or Accessible, and TuiGateway
/// falls back to CliGateway-style line output.
pub struct TuiGateway {
    ctx: Arc<GatewayContext>,
    render_mode: RenderMode,
    session_id: String,
    model: String,
}

impl TuiGateway {
    pub fn new(ctx: Arc<GatewayContext>, accessible: bool, model: impl Into<String>) -> Self {
        let detected = detect_capabilities();
        let render_mode = render::apply_accessible_flag(detected, accessible);
        Self {
            ctx,
            render_mode,
            session_id: uuid::Uuid::new_v4().to_string(),
            model: model.into(),
        }
    }
}

impl Gateway for TuiGateway {
    fn name(&self) -> &str {
        "tui"
    }

    fn render_mode(&self) -> RenderMode {
        self.render_mode
    }

    fn run(&self) -> Pin<Box<dyn Future<Output = Result<(), GatewayError>> + Send + '_>> {
        Box::pin(async move {
            match self.render_mode {
                RenderMode::Tui => self.run_tui().await,
                RenderMode::Accessible | RenderMode::Plain => {
                    // Degrade to line-based output via CliGateway logic.
                    let cli = crate::cli::CliGateway::new(Arc::clone(&self.ctx), self.render_mode);
                    cli.run().await
                }
            }
        })
    }
}

impl TuiGateway {
    async fn run_tui(&self) -> Result<(), GatewayError> {
        // Set up the terminal.
        enable_raw_mode().map_err(GatewayError::Io)?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen).map_err(GatewayError::Io)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).map_err(GatewayError::Io)?;

        let result = self.event_loop(&mut terminal).await;

        // Always restore the terminal, even on error.
        disable_raw_mode().ok();
        execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
        terminal.show_cursor().ok();

        result
    }

    async fn event_loop(
        &self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<(), GatewayError> {
        let size = terminal.size().map_err(GatewayError::Io)?;
        let mut app = App::new(&self.session_id, &self.model);
        app = app.update(Msg::Resize {
            cols: size.width,
            rows: size.height,
        });

        let mut input = Input::new();

        // Channel for agent events during an active run.
        let mut agent_rx: Option<mpsc::Receiver<AgentEvent>> = None;

        loop {
            // Draw the frame.
            terminal
                .draw(|frame| {
                    let areas =
                        SplitPane::split(frame.area(), app.layout_mode, app.tool_panel_visible);

                    ChatView::render(frame, areas.chat, &app.messages, app.scroll_offset);

                    InputBar::render(
                        frame,
                        areas.input,
                        &input,
                        app.status.thinking,
                        app.pending_approval.as_ref(),
                    );

                    if app.tool_panel_visible
                        && let Some(panel_area) = areas.tool_panel
                    {
                        ToolPanel::render(frame, panel_area, &app.tool_calls);
                    }

                    StatusBar::render(frame, areas.status, &app.status);
                })
                .map_err(GatewayError::Io)?;

            // Poll for events with a short timeout so we can check agent_rx.
            let keyboard_event = tokio::task::block_in_place(|| {
                if event::poll(Duration::from_millis(50)).unwrap_or(false) {
                    event::read().ok()
                } else {
                    None
                }
            });

            if let Some(evt) = keyboard_event {
                app = handle_keyboard_event(evt, app, &mut input, &self.ctx, &mut agent_rx).await;
            }

            // Drain agent events without blocking.
            {
                let mut should_close = false;
                if let Some(rx) = agent_rx.as_mut() {
                    while let Ok(agent_event) = rx.try_recv() {
                        if matches!(&agent_event, AgentEvent::Completed | AgentEvent::Failed(_)) {
                            should_close = true;
                        }
                        app = handle_agent_event(agent_event, app);
                    }
                }
                if should_close {
                    agent_rx = None;
                }
            }

            if app.should_quit {
                break;
            }
        }

        Ok(())
    }
}

async fn handle_keyboard_event(
    evt: Event,
    mut app: App,
    input: &mut Input,
    ctx: &Arc<GatewayContext>,
    agent_rx: &mut Option<mpsc::Receiver<AgentEvent>>,
) -> App {
    match evt {
        Event::Key(key) => {
            // Ctrl+C / Ctrl+Q → quit
            if key.modifiers == KeyModifiers::CONTROL
                && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('q'))
            {
                return app.update(Msg::Quit);
            }

            // Ctrl+Enter or Alt+Enter → submit message
            if (key.modifiers == KeyModifiers::CONTROL || key.modifiers == KeyModifiers::ALT)
                && key.code == KeyCode::Enter
            {
                let text: String = input.text();
                let trimmed = text.trim();
                if trimmed.is_empty() || app.status.thinking {
                    return app;
                }

                let user_text = trimmed.to_string();
                input.clear();
                app = app.update(Msg::UserSubmit(user_text.clone()));

                // Spawn agent turn
                let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(64);
                *agent_rx = Some(event_rx);

                let ctx_clone = Arc::clone(ctx);
                let session_id = app.status.session_id.clone();
                tokio::spawn(async move {
                    let mut agent = ctx_clone.build_agent(event_tx);
                    if let Err(e) = agent.run(&session_id, user_text).await {
                        tracing::warn!("agent run failed: {e}");
                    }
                });

                return app;
            }

            // Approval pending: y/n keys
            if let Some(pending) = app.pending_approval.take() {
                let approved = matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'));
                pending.tx.send(approved).ok();
                return app;
            }

            // Tab → toggle tool panel
            if key.code == KeyCode::Tab {
                return app.update(Msg::ToggleToolPanel);
            }

            // Page up/down / arrow scrolling
            if key.code == KeyCode::PageUp {
                return app.update(Msg::ScrollUp);
            }
            if key.code == KeyCode::PageDown {
                return app.update(Msg::ScrollDown);
            }

            // Forward all other keys to the input editor
            input.input(&evt);
        }
        Event::Resize(cols, rows) => {
            app = app.update(Msg::Resize { cols, rows });
        }
        _ => {}
    }

    app
}

fn handle_agent_event(event: AgentEvent, app: App) -> App {
    match event {
        AgentEvent::Text { content } => app.update(Msg::AssistantText(content)),
        AgentEvent::ToolCalled { id, name, .. } => app.update(Msg::ToolStarted { id, name }),
        AgentEvent::ToolResult {
            id,
            content,
            is_error,
        } => {
            if is_error {
                app.update(Msg::ToolError { id, error: content })
            } else {
                app.update(Msg::ToolDone {
                    id,
                    output: content,
                })
            }
        }
        AgentEvent::ApprovalRequested {
            call_id,
            tool_name,
            args,
            tx,
            ..
        } => app.update(Msg::ApprovalRequested {
            call_id,
            tool_name,
            args,
            tx,
        }),
        AgentEvent::Completed => app.update(Msg::AgentDone),
        AgentEvent::Failed(msg) => app.update(Msg::AgentFailed(msg)),
        _ => app,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use talon_llm::MockProvider;

    use app::Msg;
    use components::{chat::ChatView, status::StatusBar};
    use layout::SplitPane;

    fn make_ctx() -> Arc<GatewayContext> {
        Arc::new(GatewayContext::new(Arc::new(MockProvider::text(
            "tui response",
            "end_turn",
        ))))
    }

    #[test]
    fn tui_gateway_name() {
        let gw = TuiGateway::new(make_ctx(), false, "claude");
        assert_eq!(gw.name(), "tui");
    }

    #[test]
    fn tui_gateway_accessible_flag() {
        let gw = TuiGateway::new(make_ctx(), true, "claude");
        // In CI (piped stdin) detected = Plain; accessible overrides to Accessible.
        // In an interactive terminal, detected = Tui; accessible overrides to Accessible.
        // Either way, if accessible=true, result must not be Tui.
        assert_ne!(gw.render_mode(), RenderMode::Tui);
    }

    /// Headless render smoke test: push a message into App, draw one frame
    /// via TestBackend, and verify the message text appears in the cell buffer.
    #[test]
    fn tui_headless_render_smoke() {
        let backend = TestBackend::new(120, 30);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");

        let mut app = App::new("sess-smoke", "claude-sonnet");
        app = app.update(Msg::Resize {
            cols: 120,
            rows: 30,
        });
        app = app.update(Msg::AssistantText("smoke test message content".to_string()));

        terminal
            .draw(|frame| {
                let areas = SplitPane::split(frame.area(), app.layout_mode, false);
                ChatView::render(frame, areas.chat, &app.messages, app.scroll_offset);
                StatusBar::render(frame, areas.status, &app.status);
            })
            .expect("draw");

        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();

        assert!(
            content.contains("smoke test message content"),
            "expected message in render buffer, got:\n{content}"
        );
        assert!(
            content.contains("claude-sonnet"),
            "expected model name in status bar"
        );
    }
}
