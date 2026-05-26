# TUI Implementation (ratatui)

> **Status:** ✅ Complete
> **Category:** Core Features

---

## 1. Why ratatui?

| Option | Verdict |
|--------|---------|
| `ink` (React TUI) | Drop — TS-only, N/A for Rust |
| `rich` (Python) | Drop — Python-only |
| `ratatui` | ✅ Use — mature Rust TUI, active community |
| `cursive` | ❌ Skip — older, less feature-rich |
| `egui` (immediate-mode GUI) | Overkill — creates a window, not terminal |

`ratatui` is the de facto standard for terminal UIs in Rust.
It handles raw terminal mode, layout, input events, and rendering.

---

## 2. Layout Design

```
┌──────────────────────────────────────────────────────┐
│ Talon v1.0.0   Profile: default   Model: claude-3   │  ← StatusBar
├──────────────────────────────────────────────────────┤
│                                                      │
│  [Assistant] Here's the Fibonacci function...        │  ← ChatPane
│                                                      │  (scrollable)
│  [Tool: read_file] src/lib.rs                        │
│  [Output] pub fn fibonacci(n: u64) -> u64 { ...      │
│                                                      │
│  [Assistant] I've implemented it. Run `cargo test`   │
│  to verify.                                          │
│                                                      │
├──────────────────────────────────────────────────────┤
│  ⚠ Approval: terminal_execute("cargo test")  [Y/n]  │  ← ApprovalBar
├──────────────────────────────────────────────────────┤
│  > _                                                 │  ← InputBar
│                                                [↵ ]  │
└──────────────────────────────────────────────────────┘
```

---

## 3. App State

```rust
// talon-gateway/src/cli/app.rs

pub struct App {
    pub messages: Vec<DisplayMessage>,
    pub input: String,
    pub input_cursor: usize,
    pub scroll_offset: u16,
    pub pending_approval: Option<ApprovalRequest>,
    pub status: AppStatus,
    pub is_streaming: bool,
}

pub enum AppStatus {
    Idle,
    Thinking,
    ToolExecuting { tool_name: String },
    WaitingApproval,
    Error(String),
}

pub struct DisplayMessage {
    pub role: DisplayRole,
    pub content: String,
    pub timestamp: DateTime<Local>,
    pub tool_name: Option<String>,
    pub is_error: bool,
}

pub enum DisplayRole {
    User,
    Assistant,
    ToolCall,
    ToolResult,
    System,
}
```

---

## 4. Event Loop

```rust
pub async fn run_tui(app_state: Arc<AppState>) -> Result<(), TuiError> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Channels
    let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(256);
    let (input_tx, mut input_rx) = mpsc::channel::<InputEvent>(32);

    let mut app = App::default();

    loop {
        // Render
        terminal.draw(|f| ui::render(f, &app))?;

        tokio::select! {
            // Terminal keyboard input
            Some(event) = input_rx.recv() => {
                match event {
                    InputEvent::Key(KeyEvent { code: KeyCode::Enter, .. }) => {
                        if !app.input.trim().is_empty() {
                            let msg = std::mem::take(&mut app.input);
                            app.input_cursor = 0;
                            app.messages.push(DisplayMessage::user(msg.clone()));
                            app.status = AppStatus::Thinking;

                            // Fire off agent run
                            let state = app_state.clone();
                            let tx = agent_tx.clone();
                            tokio::spawn(async move {
                                state.run_agent_streaming(msg, tx).await.ok();
                            });
                        }
                    }

                    InputEvent::Key(KeyEvent { code: KeyCode::Char('y'), modifiers: KeyModifiers::NONE })
                        if matches!(app.status, AppStatus::WaitingApproval) =>
                    {
                        app.approve_pending().await;
                    }

                    InputEvent::Key(KeyEvent { code: KeyCode::Char('n'), .. })
                        if matches!(app.status, AppStatus::WaitingApproval) =>
                    {
                        app.deny_pending().await;
                    }

                    InputEvent::Key(KeyEvent { code: KeyCode::Char(c), .. }) => {
                        app.input.insert(app.input_cursor, c);
                        app.input_cursor += 1;
                    }

                    InputEvent::Key(KeyEvent { code: KeyCode::Backspace, .. }) => {
                        if app.input_cursor > 0 {
                            app.input_cursor -= 1;
                            app.input.remove(app.input_cursor);
                        }
                    }

                    InputEvent::Key(KeyEvent { code: KeyCode::Char('c'), modifiers: KeyModifiers::CONTROL }) => {
                        break;  // Ctrl+C = quit
                    }

                    InputEvent::Scroll(delta) => {
                        app.scroll_offset = app.scroll_offset.saturating_add_signed(delta);
                    }

                    _ => {}
                }
            }

            // Agent events (streaming)
            Some(event) = agent_rx.recv() => {
                match event {
                    AgentEvent::TextDelta(chunk) => {
                        app.append_streaming_chunk(chunk);
                        app.is_streaming = true;
                    }
                    AgentEvent::ToolCall { name, .. } => {
                        app.status = AppStatus::ToolExecuting { tool_name: name.clone() };
                        app.messages.push(DisplayMessage::tool_call(name));
                    }
                    AgentEvent::ToolResult { output, is_error, .. } => {
                        app.messages.push(DisplayMessage::tool_result(output, is_error));
                        app.status = AppStatus::Thinking;
                    }
                    AgentEvent::ApprovalRequired { tool, id, .. } => {
                        app.pending_approval = Some(ApprovalRequest { tool, id });
                        app.status = AppStatus::WaitingApproval;
                    }
                    AgentEvent::Done { .. } => {
                        app.status = AppStatus::Idle;
                        app.is_streaming = false;
                        app.finalize_streaming_message();
                    }
                    AgentEvent::Error(e) => {
                        app.status = AppStatus::Error(e.to_string());
                    }
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    Ok(())
}
```

---

## 5. Rendering

```rust
pub fn render(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),   // status bar
            Constraint::Min(0),      // chat pane
            Constraint::Length(if app.pending_approval.is_some() { 3 } else { 0 }),
            Constraint::Length(3),   // input bar
        ])
        .split(f.area());

    render_status_bar(f, app, chunks[0]);
    render_chat_pane(f, app, chunks[1]);
    if let Some(approval) = &app.pending_approval {
        render_approval_bar(f, approval, chunks[2]);
    }
    render_input_bar(f, app, chunks[3]);
}

fn render_chat_pane(f: &mut Frame, app: &App, area: Rect) {
    let messages: Vec<ListItem> = app.messages.iter().map(|msg| {
        let (prefix, style) = match msg.role {
            DisplayRole::User      => ("You", Style::default().fg(Color::Cyan)),
            DisplayRole::Assistant => ("Talon", Style::default().fg(Color::Green)),
            DisplayRole::ToolCall  => ("Tool", Style::default().fg(Color::Yellow)),
            DisplayRole::ToolResult=> ("Output", Style::default().fg(Color::DarkGray)),
            DisplayRole::System    => ("System", Style::default().fg(Color::Magenta)),
        };
        ListItem::new(format!("[{}] {}", prefix, msg.content))
            .style(style)
    }).collect();

    let list = List::new(messages)
        .block(Block::default().borders(Borders::ALL).title("Chat"))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    f.render_widget(list, area);
}

fn render_input_bar(f: &mut Frame, app: &App, area: Rect) {
    let input = Paragraph::new(app.input.as_str())
        .style(Style::default().fg(Color::White))
        .block(Block::default().borders(Borders::ALL).title(
            match &app.status {
                AppStatus::Idle => "Input",
                AppStatus::Thinking => "Thinking…",
                AppStatus::ToolExecuting { tool_name } =>
                    format!("Running {}…", tool_name).leak(),
                AppStatus::WaitingApproval => "Approval required",
                AppStatus::Error(e) => format!("Error: {}", e).leak(),
            }
        ));
    f.render_widget(input, area);

    // Show cursor
    f.set_cursor_position((
        area.x + app.input_cursor as u16 + 1,
        area.y + 1,
    ));
}
```

---

## 6. Slash Commands

The TUI recognizes slash commands entered in the input bar:

```rust
fn handle_slash_command(cmd: &str) -> Option<SlashCommand> {
    match cmd.trim() {
        "/spec"   => Some(SlashCommand::LoadSkill("spec")),
        "/plan"   => Some(SlashCommand::LoadSkill("plan")),
        "/review" => Some(SlashCommand::LoadSkill("requesting-code-review")),
        "/clear"  => Some(SlashCommand::ClearHistory),
        "/profile"=> Some(SlashCommand::ShowProfile),
        "/quit" | "/q" => Some(SlashCommand::Quit),
        cmd if cmd.starts_with("/model ") => {
            let model = cmd.trim_start_matches("/model ").trim();
            Some(SlashCommand::SetModel(model.to_string()))
        }
        _ => None,
    }
}
```
---

## Related Documents

### See Also
- [Gateway Architecture](../02_Architecture/18_Gateway_MultiChannel_Architecture.md)
- [Streaming & Realtime Output](31a_Streaming_And_Realtime_Output.md)
- [Agent Loop Implementation](29_Agent_Loop_Implementation.md)

