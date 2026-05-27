# 78 — TUI Technology Selection: Ratatui + Crossterm

> **Decision:** Ratatui (immediate mode) + Crossterm (backend) + MVU architecture pattern
> **Status:** ★★★★★ Selected
> **Confidence:** HIGH — industry-validated, Rust-native, best ecosystem

---

## Decision

Talon's TUI will be built on **Ratatui + Crossterm**, using a **Model-View-Update (MVU)** architecture inspired by Bubbletea (Go).

### Why Ratatui

1. **Rust-native** — no FFI, no runtime overhead, compiles into Talon's single binary
2. **Immediate mode** — perfect for streaming LLM output that changes every frame
3. **Largest ecosystem** — 3,700+ dependent crates, actively maintained
4. **Async-compatible** — crossterm's EventStream integrates with tokio
5. **Proven** — AI agent TUIs (tenere, oatmeal) already built with it
6. **Layout system** — `Layout::split()` handles split panes natively

### Why NOT Alternatives

| Alternative | Rejection Reason |
|---|---|
| Ink (React/TS) | Node.js dependency — Talon is Rust |
| Textual (Python) | Python only |
| Bubbletea (Go) | Go only — but we adopt its MVU pattern |
| Cursive (Rust) | Smaller ecosystem, less flexible, retained mode |

### Why Crossterm Over Termion/Termwiz

- Cross-platform (Windows + Unix)
- Async tokio EventStream
- Most actively maintained
- Ratatui's default backend

---

## Architecture: MVU on Ratatui

```rust
// Simplified MVU loop
struct App {
    // Model — all application state
    chat: ChatState,
    input: InputState,
    tools: ToolPanelState,
    layout: LayoutMode,
}

enum Msg {
    // User events
    Key(KeyEvent),
    Resize(u16, u16),
    // Agent events (from async channels)
    StreamChunk(String),
    StreamEnd,
    ToolStart { name: String, id: String },
    ToolOutput { id: String, output: String },
    ToolEnd { id: String },
    // System
    Tick,
}

impl App {
    fn update(&mut self, msg: Msg) -> Option<Cmd> {
        match msg {
            Msg::StreamChunk(text) => {
                self.chat.append_streaming(&text);
                None
            }
            Msg::Key(KeyEvent { code: KeyCode::Enter, .. }) => {
                let input = self.input.take();
                Some(Cmd::SendMessage(input))
            }
            Msg::Resize(w, h) => {
                self.layout = LayoutMode::from_dimensions(w, h);
                None
            }
            // ...
        }
    }

    fn view(&self, frame: &mut Frame) {
        let chunks = self.layout.split(frame.area());
        // Render each component into its chunk
        ChatView::render(&self.chat, frame, chunks[0]);
        ToolPanel::render(&self.tools, frame, chunks[1]);
        InputBar::render(&self.input, frame, chunks[2]);
        StatusBar::render(self, frame, chunks[3]);
    }
}
```

### Async Integration

```
┌─────────────────────────────────────────┐
│  tokio runtime                          │
│                                         │
│  ┌──────────┐     mpsc        ┌──────┐ │
│  │ LLM      │────channels────▶│      │ │
│  │ streaming │                 │ MVU  │ │
│  ├──────────┤                 │ Loop │ │
│  │ Tool     │────channels────▶│      │ │
│  │ executor │                 │      │ │
│  ├──────────┤                 │      │ │
│  │ crossterm│────EventStream─▶│      │ │
│  │ events   │                 │      │ │
│  └──────────┘                 └──────┘ │
└─────────────────────────────────────────┘
```

All async events flow through `mpsc` channels into a single MVU loop. No shared mutable state.

---

## Component Design

### ChatView
- Scrollable message history
- Streaming markdown rendering (comrak AST → ratatui widgets)
- Syntax-highlighted code blocks (syntect)
- Tool call indicators inline with messages

### InputBar
- Multi-line input via `tui-textarea`
- Command history (up/down arrows)
- Slash command autocomplete
- `@` file reference completion

### ToolPanel
- Collapsible side/bottom panel
- Shows active tool executions with spinners
- Tool output with syntax highlighting
- Expand/collapse individual tool calls

### StatusBar
- Current model + provider
- Token count (input/output)
- Session name
- Connection status

### Adaptive Layout

```
< 80 cols: Compact (stacked)     ≥ 120 cols: Full (side-by-side)
┌────────────────────┐           ┌──────────────┬──────────┐
│     ChatView       │           │              │  Tool    │
│                    │           │   ChatView   │  Panel   │
│                    │           │              │          │
├────────────────────┤           │              │          │
│     InputBar       │           ├──────────────┤          │
├────────────────────┤           │   InputBar   │          │
│     StatusBar      │           ├──────────────┴──────────┤
└────────────────────┘           │        StatusBar        │
                                 └─────────────────────────┘
```

---

## Essential Crates

| Crate | Purpose |
|---|---|
| `ratatui` | TUI framework (immediate mode rendering) |
| `crossterm` | Terminal backend (events, raw mode, ANSI) |
| `comrak` | CommonMark/GFM parser → AST for markdown rendering |
| `syntect` | Syntax highlighting (Sublime grammars) |
| `tui-textarea` | Multi-line text input with editing |
| `ratatui-image` | Inline images (Sixel, Kitty, iTerm2, halfblocks) |
| `similar` | Diff algorithm for showing file changes |
| `indicatif` | Progress bars/spinners (non-TUI fallback mode) |
| `inquire` | Interactive prompts (non-TUI fallback mode) |
| `strip-ansi-escapes` | Clean output for logging/accessibility |
| `unicode-width` | Correct layout with CJK/emoji characters |

---

## Non-TUI Fallback

Talon MUST work without TUI for:
- Piped input/output (`echo "fix bug" | talon`)
- CI/CD environments
- Screen readers (`--accessible` flag)
- `$TERM=dumb` terminals
- `NO_COLOR` environments

Fallback: simple line-by-line output using `indicatif` spinners and `inquire` prompts.

---

## Web Hybrid (Future)

Optional WebSocket server that streams ANSI output to xterm.js in browser:
- Same Ratatui rendering → capture ANSI → stream to web client
- Zero-install browser access
- Reference: Zellij's web client approach

---

## Reference Implementations to Study

1. **OpenCode** (Go/Bubbletea) — closest to Talon's target UX. Full TUI with split panes, vim keys, markdown, tool execution display.
2. **Amazon Q CLI** (Rust/crossterm) — Rust-native AI CLI, custom rendering.
3. **tenere** (Rust/Ratatui) — Simple AI chat TUI, good starting point.
4. **Claude Code** (TS/Ink) — UX benchmark for streaming, tool calls, markdown. Study the UX, not the tech.
