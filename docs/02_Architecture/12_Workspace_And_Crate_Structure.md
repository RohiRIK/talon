# Cargo Workspace & Crate Structure

> **Last corrected:** dogfood pass 3

> **Status:** ✅ Complete
> **Category:** Architecture
> **Last corrected:** dogfood pass 3

---

## 1. Workspace Root `Cargo.toml`

```toml
[workspace]
resolver = "2"
members = [
    "talon",           # binary
    "crates/talon-core",
    "crates/talon-tools",
    "crates/talon-memory",
    "crates/talon-llm",
    "crates/talon-gateway",
    "crates/talon-plugins",
]

[workspace.package]
edition = "2024"
version = "0.1.0"
authors = ["Talon Contributors"]
license = "MIT"
repository = "https://github.com/RohiRIK/talon"

[workspace.dependencies]
# Async
tokio          = { version = "1", features = ["full"] }
futures        = "0.3"
async-trait    = "0.1"
tokio-stream   = "0.1"

# HTTP
reqwest        = { version = "0.12", features = ["json", "stream"] }
axum           = { version = "0.7", features = ["ws"] }

# Serialization
serde          = { version = "1", features = ["derive"] }
serde_json     = "1"
schemars       = { version = "0.8", features = ["derive"] }

# Database
rusqlite       = { version = "0.31", features = ["bundled", "vtab", "functions"] }

# Error handling
thiserror      = "1"
anyhow         = "1"

# Logging
tracing        = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# Config
config         = "0.14"
toml           = "0.8"

# Templates
askama         = { version = "0.12", features = ["with-axum"] }

# Cron
tokio-cron-scheduler = "0.10"
cron           = "0.12"

# TUI
ratatui        = "0.26"
crossterm      = "0.27"

# LLM
async-openai   = "0.23"
tiktoken-rs    = "0.5"

# Platform gateways
teloxide       = { version = "0.12", features = ["macros"] }
serenity       = { version = "0.12", features = ["client", "gateway", "model"] }

# Browser
chromiumoxide  = { version = "0.6", features = ["tokio-runtime"] }

# WASM plugins
wasmtime       = "18"

# Utilities
uuid           = { version = "1", features = ["v4"] }
chrono         = { version = "0.4", features = ["serde"] }
humantime      = "2"
regex          = "1"
walkdir        = "2"
notify         = "6"   # file-system watcher for skill hot-reload
bollard        = "0.17" # Docker SDK
```

---

## 2. Directory Layout

```
talon/
├── Cargo.toml              # workspace root
├── Cargo.lock
├── talon/                 # binary crate
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       └── cli.rs
├── crates/
│   ├── talon-core/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── agent.rs        # Agent struct
│   │       ├── loop_.rs        # run_loop()
│   │       ├── context.rs      # ContextBuilder
│   │       ├── turn.rs         # Turn / Delta types
│   │       ├── approval.rs     # ApprovalMembrane
│   │       ├── limits.rs       # token/iteration guards
│   │       └── prompts/        # .askama templates
│   ├── talon-llm/
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── provider.rs     # LlmProvider trait
│   │       ├── openai.rs       # OpenAI-compatible
│   │       ├── anthropic.rs    # Anthropic direct
│   │       ├── ollama.rs       # Local via OpenAI compat
│   │       └── message.rs      # Message / ContentBlock
│   ├── talon-memory/
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── store.rs        # MemoryStore (SQLite)
│   │       ├── schema.rs       # CREATE TABLE / migrations
│   │       ├── fts.rs          # FTS5 search
│   │       ├── skills.rs       # SkillStore
│   │       ├── user_model.rs   # USER.md + mem0 bridge
│   │       └── cron_store.rs   # CronJob persistence
│   ├── talon-tools/
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── registry.rs     # ToolRegistry
│   │       ├── tool.rs         # Tool trait
│   │       ├── context.rs      # ToolContext
│   │       ├── terminal.rs
│   │       ├── file_ops.rs
│   │       ├── search.rs
│   │       ├── web.rs
│   │       ├── browser.rs
│   │       ├── memory_tools.rs
│   │       ├── skill_tools.rs
│   │       ├── cron_tools.rs
│   │       ├── image_gen.rs
│   │       ├── tts.rs
│   │       └── send_message.rs
│   ├── talon-gateway/
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── gateway.rs      # Gateway trait
│   │       ├── router.rs       # incoming dispatch
│   │       ├── cli.rs
│   │       ├── telegram.rs
│   │       ├── discord.rs
│   │       ├── http.rs         # ACP + webhook endpoint
│   │       └── media.rs        # file upload helpers
│   └── talon-plugins/
│       └── src/
│           ├── lib.rs
│           ├── host.rs         # PluginHost (wasmtime)
│           └── wasm_tool.rs    # WasmTool wrapper
├── docs/                   # ← this directory
└── tests/
    ├── integration/
    └── e2e/
```

---

## 3. Core Traits (Summary)

```rust
// talon-tools/src/tool.rs
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters(&self) -> RootSchema;  // schemars
    async fn execute(&self, ctx: ToolContext) -> Result<ToolResult, ToolError>;
}

// talon-llm/src/provider.rs
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        req: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Delta, LlmError>> + Send>>, LlmError>;
    fn supports_vision(&self) -> bool { false }
    fn supports_tool_use(&self) -> bool { true }
}

// talon-gateway/src/gateway.rs
#[async_trait]
pub trait Gateway: Send + Sync {
    fn id(&self) -> &str;
    async fn send(&self, msg: OutboundMessage) -> Result<(), GatewayError>;
    async fn send_stream(&self, stream: MessageStream) -> Result<(), GatewayError>;
    async fn run(&self, router: Arc<MessageRouter>) -> Result<(), GatewayError>;
}
```

---

## 4. Feature Flags

```toml
# talon-core/Cargo.toml
[features]
default     = []
voice       = ["whisper-rs", "rodio"]
vision      = ["talon-tools/browser"]
embeddings  = ["fastembed"]
evolution   = []   # enables trajectory collection
```

Single binary with `--features voice,vision,embeddings` for full install.
---

## Related Documents

### Depends On
- [System Architecture Overview](11_System_Architecture_Overview.md)

### Used By
- [Build System / Cargo Workspace](../08_DevOps/60_Build_System_Cargo_Workspace.md)
- [Core Agent Loop Design](13_Core_Agent_Loop_Design.md)

### See Also
- [Error Handling Strategy](../06_Concurrency/54_Error_Handling_Strategy.md)
- [CI/CD Pipeline](../08_DevOps/62_CI_CD_Pipeline.md)

