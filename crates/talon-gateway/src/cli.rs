use std::future::Future;
use std::io::Write as _;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use talon_core::events::AgentEvent;

use crate::Gateway;
use crate::{GatewayContext, GatewayError, RenderMode, normalize::normalize_markdown};

/// Single-user interactive CLI gateway.
///
/// Starts a REPL: reads one line from stdin, runs the agent, prints the response.
/// A spinner is shown while the agent thinks; Dangerous tool approvals are prompted
/// via stderr so they don't interfere with the spinner.
pub struct CliGateway {
    ctx: Arc<GatewayContext>,
    render_mode: RenderMode,
    session_id: String,
}

impl CliGateway {
    pub fn new(ctx: Arc<GatewayContext>, render_mode: RenderMode) -> Self {
        Self {
            ctx,
            render_mode,
            session_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    /// Run one agent turn, printing the response to stdout.
    ///
    /// Exposed as `pub` so callers can use single-turn mode without the REPL loop.
    pub async fn run_turn_pub(&self, user_message: String) -> Result<(), GatewayError> {
        self.run_turn(user_message).await
    }

    /// Run one agent turn and return the (normalized) response text without printing.
    ///
    /// Used by tests to verify response content without capturing stdout.
    #[cfg(test)]
    async fn run_turn_text(&self, user_message: String) -> Result<String, GatewayError> {
        let render_mode = self.render_mode;
        let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(64);

        let event_handle = tokio::spawn(async move {
            let mut last_text = String::new();
            while let Some(event) = event_rx.recv().await {
                match event {
                    AgentEvent::Text { content } => last_text = content,
                    AgentEvent::Completed | AgentEvent::Failed(_) => break,
                    AgentEvent::ApprovalRequested { tx, .. } => {
                        tx.send(false).ok();
                    }
                    _ => {}
                }
            }
            last_text
        });

        let mut agent = self.ctx.build_agent(event_tx);
        agent
            .run(&self.session_id, user_message)
            .await
            .map_err(|e| GatewayError::Agent(e.to_string()))?;

        let text = event_handle.await.unwrap_or_default();
        Ok(normalize_markdown(&text, render_mode))
    }

    async fn run_turn(&self, user_message: String) -> Result<(), GatewayError> {
        let render_mode = self.render_mode;
        let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(64);

        // Spinner — indicatif is internally Arc-backed; cloning is cheap.
        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap_or_else(|_| ProgressStyle::default_spinner()),
        );
        spinner.set_message("thinking…");
        spinner.enable_steady_tick(Duration::from_millis(100));

        let spinner_clone = spinner.clone();

        // Collect the assistant response and handle approvals on a sibling task.
        let event_handle = tokio::spawn(async move {
            let mut last_text = String::new();
            while let Some(event) = event_rx.recv().await {
                match event {
                    AgentEvent::Text { content } => {
                        last_text = content;
                    }
                    AgentEvent::ToolCalled { ref name, .. } => {
                        spinner_clone.set_message(format!("running {name}…"));
                    }
                    AgentEvent::ApprovalRequested {
                        tool_name,
                        args,
                        tx,
                        ..
                    } => {
                        spinner_clone.suspend(|| {
                            eprint!("\n[talon] approve {tool_name}({args})? [y/n]: ");
                            let _ = std::io::stderr().flush();
                        });

                        let answer = tokio::task::block_in_place(|| {
                            let mut s = String::new();
                            let _ = std::io::stdin().read_line(&mut s);
                            s
                        });

                        tx.send(answer.trim().eq_ignore_ascii_case("y")).ok();
                    }
                    AgentEvent::ToolResult { is_error: true, .. } => {
                        spinner_clone.set_message("tool error — continuing…");
                    }
                    AgentEvent::ToolResult { .. } => {}
                    AgentEvent::Completed | AgentEvent::Failed(_) => break,
                    _ => {}
                }
            }
            last_text
        });

        let mut agent = self.ctx.build_agent(event_tx);
        let run_result = agent.run(&self.session_id, user_message).await;

        // Let event handler drain.
        let response_text = event_handle.await.unwrap_or_default();

        spinner.finish_and_clear();

        run_result.map_err(|e| GatewayError::Agent(e.to_string()))?;

        if !response_text.is_empty() {
            let output = normalize_markdown(&response_text, render_mode);
            println!("{}", output.trim_end());
        }

        Ok(())
    }
}

impl Gateway for CliGateway {
    fn name(&self) -> &str {
        "cli"
    }

    fn render_mode(&self) -> RenderMode {
        self.render_mode
    }

    fn run(&self) -> Pin<Box<dyn Future<Output = Result<(), GatewayError>> + Send + '_>> {
        Box::pin(async move {
            let stdin = tokio::io::stdin();
            let mut reader = BufReader::new(stdin);
            let mut line = String::new();

            print_banner();

            loop {
                print!("> ");
                let _ = std::io::stdout().flush();
                line.clear();

                let n = reader
                    .read_line(&mut line)
                    .await
                    .map_err(GatewayError::Io)?;

                if n == 0 {
                    // EOF — clean exit
                    break;
                }

                let trimmed = line.trim();

                if trimmed.is_empty() {
                    continue;
                }

                match trimmed {
                    "/quit" | "/exit" | "exit" | "quit" => break,
                    "/help" => {
                        println!("Commands: /quit  /help");
                        println!("Or type any message to chat with the agent.");
                        continue;
                    }
                    _ => {}
                }

                if let Err(e) = self.run_turn(trimmed.to_string()).await {
                    eprintln!("[talon] error: {e}");
                }

                println!();
            }

            Ok(())
        })
    }
}

fn print_banner() {
    println!("Talon — TRUST it. It's RUST.");
    println!("Type /help for commands, /quit to exit.\n");
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use talon_llm::MockProvider;

    fn make_ctx() -> Arc<GatewayContext> {
        Arc::new(GatewayContext::new(Arc::new(MockProvider::text(
            "hello from mock",
            "end_turn",
        ))))
    }

    #[test]
    fn cli_gateway_name() {
        let gw = CliGateway::new(make_ctx(), RenderMode::Plain);
        assert_eq!(gw.name(), "cli");
    }

    #[test]
    fn cli_gateway_render_mode() {
        let gw = CliGateway::new(make_ctx(), RenderMode::Accessible);
        assert_eq!(gw.render_mode(), RenderMode::Accessible);
    }

    #[tokio::test]
    async fn cli_gateway_run_turn_with_mock_provider() {
        let gw = CliGateway::new(make_ctx(), RenderMode::Plain);
        let result = gw.run_turn("hello".to_string()).await;
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }

    #[tokio::test]
    async fn cli_gateway_roundtrip_returns_mock_text() {
        let gw = CliGateway::new(make_ctx(), RenderMode::Plain);
        let text = gw
            .run_turn_text("ping".to_string())
            .await
            .expect("roundtrip");
        assert!(
            text.contains("hello from mock"),
            "expected mock response text, got: {text:?}"
        );
    }

    #[tokio::test]
    async fn cli_gateway_roundtrip_accessible_strips_nothing_for_plain_text() {
        let gw = CliGateway::new(make_ctx(), RenderMode::Accessible);
        let text = gw
            .run_turn_text("ping".to_string())
            .await
            .expect("roundtrip");
        // MockProvider returns plain text — accessible mode leaves it unchanged.
        assert!(!text.is_empty());
    }
}
