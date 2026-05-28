# Gateway & Multi-Channel Architecture

> **Status:** ✅ Built (Phase 4 complete, 2026-05-28)
> **Category:** Architecture

---

## 1. Design Principle

The gateway layer is **thin**. Its only jobs are:
1. Receive input from a channel (Telegram message, HTTP POST, CLI keystroke, TUI keypress)
2. Build a fresh `Agent` from `GatewayContext`
3. Forward `AgentEvent` stream back to that channel

All business logic lives in `talon-core`. Gateways are interchangeable and degrade gracefully.

---

## 2. Architecture Diagram

```
  User (Telegram) ──► TelegramGateway ─┐
  User (browser)  ──► HttpGateway     ─┤
  User (terminal) ──► TuiGateway      ─┤── GatewayContext::build_agent()
  User (terminal) ──► CliGateway      ─┘         │
                                                   ▼
                                        Agent::run(session_id, text)
                                                   │
                                          mpsc::Sender<AgentEvent>
                                                   │
                  ┌────────────────────────────────┴──────────┐
                  │  AgentEvent variants handled per-gateway  │
                  │  Text       → print / send / render       │
                  │  ToolCalled → spinner / panel entry       │
                  │  ToolResult → update panel                │
                  │  ApprovalRequested → prompt user          │
                  │  Completed  → finalize                    │
                  └────────────────────────────────────────────┘
```

---

## 3. Core Types

### Gateway trait (`crates/talon-gateway/src/lib.rs`)

```rust
// Object-safe: returns Pin<Box<dyn Future>> instead of async fn
// so it can be stored as Arc<dyn Gateway> in GatewayRegistry.
pub trait Gateway: Send + Sync {
    fn name(&self) -> &str;
    fn render_mode(&self) -> RenderMode;
    fn run(&self) -> Pin<Box<dyn Future<Output = Result<(), GatewayError>> + Send + '_>>;
}
```

### RenderMode

```rust
pub enum RenderMode {
    Plain,       // raw text, no colour — CI, piped stdin, $TERM=dumb
    Accessible,  // line-by-line, no escapes — screen readers, --accessible
    Tui,         // full ratatui TUI — interactive terminal only
}
```

Auto-detected at startup via `detect_capabilities()` (`tui/render.rs`):
checks `NO_COLOR`, `TERM`, `isatty`, `TALON_ACCESSIBLE` in order.

### GatewayContext

```rust
// Shared infra — constructed once in main(), passed to all gateways.
pub struct GatewayContext {
    pub provider: Arc<dyn LlmProvider>,
    pub tools: Vec<Arc<dyn Tool>>,
    pub db: Option<Arc<Database>>,
}

impl GatewayContext {
    // Builds a fresh Agent + ToolDispatcher per request.
    // Avoids needing Clone on the dispatcher.
    pub fn build_agent(&self, event_tx: mpsc::Sender<AgentEvent>) -> Agent { ... }
}
```

---

## 4. LLM Provider Selection

Selected via `TALON_LLM_PROVIDER` env var at startup (`talon/src/main.rs`):

| Value | Provider | Auth |
|-------|----------|------|
| `anthropic` (default) | `AnthropicProvider` | `TALON_LLM_API_KEY` or OS keychain |
| `github-copilot` or `copilot` | `GitHubCopilotProvider` | `GITHUB_TOKEN` or `gh auth token` |

Key-less providers (GitHub Copilot, ClaudeCode) skip the API-key gate entirely.

Model override: `TALON_LLM_MODEL=claude-sonnet-4.6` (each provider has its own default).

---

## 5. CLI Gateway (`crates/talon-gateway/src/cli.rs`)

Single-user REPL over stdin/stdout. Falls back to `CliGateway` automatically
when `TuiGateway` detects a non-interactive terminal.

- `indicatif` spinner while agent thinks
- `/quit`, `/help` commands
- `--message "..."` flag for single-turn non-interactive mode
- Approval prompts written to stderr (doesn't interrupt the spinner)

---

## 6. TUI Gateway (`crates/talon-gateway/src/tui/`)

Full ratatui terminal UI. MVU (Model-View-Update, Elm-style) pattern.

```
App (model) ──► update(Msg) → App   ← pure, no side effects
                                         │
                                    render(Frame)   ← also pure
```

### Components

| File | Widget | Notes |
|------|--------|-------|
| `components/chat.rs` | `ChatView` | Streaming markdown, inline bold/italic/code |
| `components/input.rs` | `InputBar` | `tui-textarea`, Ctrl+Enter to submit |
| `components/tools.rs` | `ToolPanel` | Collapsible (Tab), icons ⠿/✓/✗ |
| `components/status.rs` | `StatusBar` | Model, session ID, tokens, `[NATIVE]` badge |
| `layout.rs` | `SplitPane` | `<120 cols` stacked, `≥120 cols` side-by-side |

### Degradation

`TuiGateway` calls `detect_capabilities()` at construction time.
If the terminal is not interactive (`Plain` or `Accessible`), it delegates
the entire run to `CliGateway` — no raw-mode is entered.

---

## 7. HTTP Gateway (`crates/talon-gateway/src/http.rs`)

Single `POST /v1/messages` endpoint via axum.

```
POST /v1/messages
Body: { "content": "hello", "session_id": "optional-uuid" }
→    { "content": "response", "session_id": "uuid" }
```

Auto-approves `Safe` tools; auto-denies `Dangerous` tools.
Callers that need approval should use TUI or CLI.

---

## 8. Telegram Gateway (`crates/talon-gateway/src/telegram.rs`)

Feature-gated: `--features talon-gateway/telegram`.

### User Auth (`UserAuth`)

First-run auto-registration pattern:

```
First message received
  → no owner on file
  → register sender's user ID
  → write ~/.talon/telegram_owner
  → all subsequent senders from other IDs → "This is a private assistant."
```

Override: `TELEGRAM_ALLOWED_USER_IDS=123456789,987654321` (comma-separated).

The handler uses `Update::filter_message().endpoint(|bot, msg|)` directly
(not `filter_map`) to avoid dptree 3-tuple injection bound limits.

### Running

```bash
TELEGRAM_BOT_TOKEN=<token> \
TALON_LLM_PROVIDER=github-copilot \
cargo run --features talon-gateway/telegram -- --gateway telegram
```

---

## 9. SendMessageTool (`crates/talon-tools/src/send_message.rs`)

Allows the agent to push outbound messages to any registered gateway channel.
Approval level: `NeedsApproval` — user must confirm before the agent sends.

```rust
pub trait MessageSink: Send + Sync {
    fn send(&self, channel_id: &str, content: &str) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}
```

---

## 10. GatewayRegistry (`crates/talon-gateway/src/registry.rs`)

```rust
pub struct GatewayRegistry {
    gateways: HashMap<ChannelId, Arc<dyn Gateway>>,
}
```

Used for multi-channel routing (Phase 5+). Currently each binary run
uses a single gateway selected by `--gateway` flag.

---

## Related Documents

- [Workspace & Crate Structure](12_Workspace_And_Crate_Structure.md)
- [Approval Membrane](17a_Approval_Membrane.md)
- [Config System](18a_Config_System.md)
- [LLM Provider Architecture](../04_Core_Features/LLM_Providers.md) ← new
