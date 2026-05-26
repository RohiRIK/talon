# Canonical Types Reference

> **Purpose:** Single source of truth for every shared type used across multiple Talon docs.
> When a type appears here, it means ALL 65 docs must use exactly this definition.
> This file exists because the dogfood audit found 3 conflicting type definitions across 20+ docs.
> Don't let it happen again.

---

## How This File Is Used

1. Before writing any code example in any doc, check this file for the canonical type
2. If you find a doc using a type that contradicts this file — it's wrong, fix the doc
3. If you believe this file needs updating — make the change here FIRST, then propagate

---

## Core Agent Types

### `Agent` struct

```rust
// Canonical location: talon-core/src/agent.rs
pub struct Agent {
    pub config: AgentConfig,
    pub llm: Arc<dyn LlmProvider>,
    pub tools: HashMap<String, Arc<dyn Tool>>,    // NOT Arc<Box<dyn Tool>>
    pub memory: Arc<dyn MemoryStore>,
    pub session: Session,
    pub approval: ApprovalMembrane,
    pub event_tx: broadcast::Sender<AgentEvent>,
}
```

**Critical:** `Arc<dyn Tool>` — never `Arc<Box<dyn Tool>>`. The extra `Box` is redundant and was flagged in audit Community 5.

### `AgentConfig` struct

```rust
// Canonical location: talon-core/src/config.rs
pub struct AgentConfig {
    pub model: String,
    pub max_tokens: usize,
    pub max_iterations: usize,
    pub system_prompt: String,
    pub profile: String,
}
```

### `AgentEvent` enum

```rust
// Canonical location: talon-core/src/events.rs
#[derive(Debug, Clone, Serialize)]
pub enum AgentEvent {
    TurnStart { session_id: String },
    LlmChunk { delta: String },
    ToolCall { name: String, params: Value },
    ToolResult { name: String, result: ToolResult },
    TurnEnd { usage: Option<TokenUsage> },
    Error { message: String },
}
```

### `AgentState` enum

```rust
// Canonical location: talon-core/src/state.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    Idle,
    Running,
    WaitingForApproval,
    Done,
    Error(String),
}
```

---

## Tool System Types

### `Tool` trait

```rust
// Canonical location: talon-core/src/tools/mod.rs
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> Value;                         // schemars::schema_for! output
    fn approval_level(&self) -> ApprovalLevel;
    async fn execute(
        &self,
        ctx: &ToolContext,
        params: Value,
    ) -> Result<ToolResult, ToolError>;
}
```

**Critical notes:**
- `name()` returns `&str` (not `String`) — a common mistake in doc code examples
- `execute()` is `async` and returns `Result<ToolResult, ToolError>` — not `ToolResult` directly, not `Result<ToolOutput, _>`

### `ToolContext` struct

```rust
// Canonical location: talon-core/src/tools/mod.rs
pub struct ToolContext {
    pub session_id: String,
    pub sender_id: Option<String>,
    pub memory: Arc<dyn MemoryStore>,
    pub profile_dir: PathBuf,
}
```

### `ToolResult` struct

```rust
// Canonical location: talon-core/src/tools/mod.rs
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
    pub metadata: Option<Value>,
}
```

**⚠️ AUDIT WARNING — This type has a history:**
- Old name: `ToolOutput` — do not use, do not alias
- Found in 45 occurrences across 20+ docs during Pass 3 audit (Community 5)
- If you see `ToolOutput` anywhere in docs → replace with `ToolResult`

```rust
// Construction helpers
impl ToolResult {
    pub fn ok(output: impl Into<String>) -> Self {
        Self { output: output.into(), is_error: false, metadata: None }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Self { output: msg.into(), is_error: true, metadata: None }
    }
}
```

### `ApprovalLevel` enum

```rust
// Canonical location: talon-core/src/tools/mod.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalLevel {
    Safe,           // Execute without asking
    NeedsApproval,  // Ask user before executing
    Dangerous,      // Warn prominently + require explicit confirmation
}
```

**⚠️ AUDIT WARNING — Three conflicting definitions were found:**
1. `AlwaysAsk | AskForDangerous | …` — WRONG, from an early draft
2. `Low | Medium | High | Critical` — WRONG, from a risk-scoring draft
3. `Safe | NeedsApproval | Dangerous` — CORRECT ✅

The correct variant names are: `Safe`, `NeedsApproval`, `Dangerous`. Any other names are wrong.

### `ToolRegistry` struct

```rust
// Canonical location: talon-core/src/tools/registry.rs
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn register(&mut self, tool: Arc<dyn Tool>);
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>>;
    pub fn all(&self) -> impl Iterator<Item = &Arc<dyn Tool>>;
    pub fn to_llm_schemas(&self) -> Vec<Value>;
}
```

### `ToolError` enum

```rust
// Canonical location: talon-core/src/tools/error.rs
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("tool not found: {0}")]
    NotFound(String),
    #[error("invalid parameters: {0}")]
    InvalidParams(String),
    #[error("execution failed: {0}")]
    ExecutionFailed(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("timeout after {0}s")]
    Timeout(u64),
}
```

---

## LLM Provider Types

### `LlmProvider` trait

```rust
// Canonical location: talon-llm/src/lib.rs
#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn complete(
        &self,
        req: LlmRequest,
    ) -> Result<LlmResponse, LlmError>;
    async fn stream(
        &self,
        req: LlmRequest,
    ) -> Result<BoxStream<'static, Result<LlmChunk, LlmError>>, LlmError>;
    async fn count_tokens(&self, req: &LlmRequest) -> Result<usize, LlmError>;
}
```

### `LlmRequest` struct

```rust
// Canonical location: talon-llm/src/types.rs
pub struct LlmRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Option<Vec<Value>>,   // JSON schemas from ToolRegistry::to_llm_schemas()
    pub max_tokens: usize,
    pub temperature: Option<f32>,
    pub system: Option<String>,
}
```

### `Message` enum

```rust
// Canonical location: talon-llm/src/types.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    User { content: Vec<ContentBlock> },
    Assistant { content: Vec<ContentBlock> },
    Tool { tool_use_id: String, content: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: Value },
    ToolResult { tool_use_id: String, content: String, is_error: bool },
}
```

### `LlmError` enum

```rust
// Canonical location: talon-llm/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("rate limited, retry after {retry_after:?}")]
    RateLimited { retry_after: Option<Duration> },
    #[error("context window exceeded: {tokens} tokens")]
    ContextWindowExceeded { tokens: usize },
    #[error("authentication failed")]
    AuthFailed,
    #[error("provider error: {0}")]
    ProviderError(String),
    #[error("network error: {0}")]
    NetworkError(#[from] reqwest::Error),
    #[error("parse error: {0}")]
    ParseError(String),
}
```

---

## Memory System Types

### `MemoryStore` trait

```rust
// Canonical location: talon-memory/src/lib.rs
#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn save_message(&self, session_id: &str, msg: &Message) -> Result<(), MemoryError>;
    async fn get_history(&self, session_id: &str, limit: usize) -> Result<Vec<Message>, MemoryError>;
    async fn search_sessions(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, MemoryError>;
    async fn load_memory_file(&self, profile_dir: &Path) -> Result<String, MemoryError>;
    async fn save_memory_file(&self, profile_dir: &Path, content: &str) -> Result<(), MemoryError>;
}
```

### `Session` struct

```rust
// Canonical location: talon-memory/src/session.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub profile: String,
}
```

### `SearchResult` struct

```rust
// Canonical location: talon-memory/src/session.rs
pub struct SearchResult {
    pub session_id: String,
    pub session_title: Option<String>,
    pub snippet: String,
    pub match_message_id: i64,
    pub messages: Vec<Message>,          // ±5 messages around match
    pub bookend_start: Vec<Message>,     // first 3 messages
    pub bookend_end: Vec<Message>,       // last 3 messages
}
```

---

## Error Hierarchy

```
AgentError (top-level, talon-core)
├── Llm(LlmError)          — from talon-llm
├── Tool(ToolError)        — from talon-core/tools
├── Memory(MemoryError)    — from talon-memory
├── Gateway(GatewayError)  — from talon-gateway
└── Config(ConfigError)    — from talon-core/config
```

All errors implement `thiserror::Error`. The three audiences are:
- **User:** `AgentError::display_for_user()` — clean, no stack traces
- **Developer:** `tracing::error!("{:?}", err)` — full structured log
- **LLM:** `ToolResult::err(err.to_string())` — error returned as tool output, LLM can recover

---

## Gateway Types

### `Gateway` trait

```rust
// Canonical location: talon-gateway/src/lib.rs
#[async_trait]
pub trait Gateway: Send + Sync {
    fn platform(&self) -> &str;
    async fn listen(&self, input_tx: mpsc::Sender<AgentInput>) -> Result<(), GatewayError>;
    async fn deliver(&self, output: AgentOutput) -> Result<(), GatewayError>;
}
```

### `AgentInput` / `AgentOutput` structs

```rust
pub struct AgentInput {
    pub session_id: String,
    pub sender_id: String,
    pub content: Vec<ContentBlock>,
    pub metadata: HashMap<String, Value>,
}

pub struct AgentOutput {
    pub session_id: String,
    pub target: DeliveryTarget,
    pub content: Vec<ContentBlock>,
    pub is_streaming: bool,
}
```

---

## Type Change Audit Checklist

Run these searches on the docs/ directory to find type violations:

```bash
# 1. ToolOutput (wrong) — should be ToolResult
grep -rn 'ToolOutput' docs/

# 2. Arc<Box<dyn Tool>> (wrong) — should be Arc<dyn Tool>
grep -rn 'Arc<Box<dyn Tool>>' docs/

# 3. Old ApprovalLevel variants
grep -rn 'AlwaysAsk\|AskForDangerous\|Low.*Medium.*High.*Critical' docs/

# 4. Old path remnants
grep -rn '~/.ernest\|~/.hermes' docs/

# 5. async build_tool_registry (wrong — registry construction is sync)
grep -rn 'async fn build_tool_registry' docs/
```

All 5 should return **zero results** if the docs are clean.

---

*Maintained by: dogfood audit system. Last clean audit: Pass 4 (score 4.9/5). Communities 4, 5, 81, 92.*
