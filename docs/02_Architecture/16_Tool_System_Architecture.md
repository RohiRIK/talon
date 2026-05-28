# Tool System Architecture

> **Last corrected:** dogfood pass 4
>
> **Status:** ✅ Complete
> **Category:** Architecture

---

## 1. The `Tool` Trait

```rust
// Canonical definition: see talon-core/src/tools/mod.rs
// Reproduced here for reference — do not diverge.
use async_trait::async_trait;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    fn approval_level(&self, args: &serde_json::Value) -> ApprovalLevel { ApprovalLevel::Safe }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> ToolResult;
}

/// Canonical result type — standardized across all docs.
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
    pub metadata: Option<serde_json::Value>,
}

/// Canonical approval levels.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ApprovalLevel {
    Safe,
    NeedsApproval,
    Dangerous,
}
```

---

## 2. ToolContext

```rust
/// Canonical ToolContext — matches talon-core/src/tools/mod.rs.
pub struct ToolContext {
    pub call_id: String,
    pub session_id: Uuid,
    pub profile_dir: PathBuf,
    pub workdir: Option<PathBuf>,
    pub approval_tx: Option<mpsc::Sender<ApprovalRequest>>,
}
```

---

## 3. ToolRegistry

```rust
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>, // Arc<dyn Tool>, NOT Arc<Box<dyn Tool>>
}

impl ToolRegistry {
    pub fn new() -> Self { Self { tools: HashMap::new() } }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), Arc::from(tool));
    }

    pub fn all_schemas(&self) -> Vec<serde_json::Value> {
        self.tools.values()
            .map(|t| serde_json::json!({
                "name": t.name(),
                "description": t.description(),
                "input_schema": t.schema()
            }))
            .collect()
    }

    pub async fn execute(
        &self,
        name: &str,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> ToolResult {
        match self.tools.get(name) {
            Some(t) => t.execute(args, ctx).await,
            None => ToolResult { output: format!("Unknown tool: {name}"), is_error: true, metadata: None },
        }
    }
}
```

---

## 4. Built-in Tools (Summary)

| Tool | Risk | Impl |
|------|------|------|
| `terminal` | Dangerous | `tokio::process::Command` |
| `read_file` | ReadOnly | `tokio::fs::read_to_string` |
| `write_file` | SafeWrite | `tokio::fs::write` |
| `patch` | SafeWrite | fuzzy diff + replace |
| `search_files` | ReadOnly | spawn `rg` |
| `web_search` | ReadOnly | reqwest → search backend |
| `web_extract` | ReadOnly | reqwest + scraper + comrak |
| `browser_navigate` | Dangerous | [chromiumoxide](../04_Core_Features/32_Browser_Tool.md) |
| `memory` (read/write) | SafeWrite | MemoryStore |
| `skill_view` | ReadOnly | SkillStore |
| `skill_manage` | SafeWrite | SkillStore + git |
| `send_message` | SafeWrite | gateway dispatch |
| `cronjob` | SafeWrite | CronStore |
| `image_gen` | SafeWrite | FAL.ai reqwest |
| `text_to_speech` | SafeWrite | OpenAI TTS |

---

## 5. JSON Schema Auto-generation

```rust
use schemars::{schema_for, JsonSchema};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TerminalParams {
    /// Shell command to execute
    pub command: String,
    /// Working directory (optional)
    pub workdir: Option<String>,
    /// Timeout in seconds (default 180)
    #[schemars(range(min = 1, max = 600))]
    pub timeout: Option<u32>,
    /// Run in background
    pub background: Option<bool>,
}

pub struct TerminalTool;

impl Tool for TerminalTool {
    fn name(&self) -> &str { "terminal" }
    fn description(&self) -> &str {
        "Execute shell commands. Returns stdout, stderr, exit code."
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::to_value(schema_for!(TerminalParams)).unwrap()
    }
    fn approval_level(&self, args: &serde_json::Value) -> ApprovalLevel { ApprovalLevel::Dangerous }
    // ...
}
```

---

## 6. WASM Plugin Tools

```rust
// talon-plugins/src/wasm_tool.rs
pub struct WasmTool {
    name: String,
    description: String,
    schema_value: serde_json::Value,
    instance: wasmtime::Instance,
    store: Arc<Mutex<wasmtime::Store<WasmState>>>,
}

#[async_trait]
impl Tool for WasmTool {
    fn name(&self) -> &str { &self.name }          // &str tied to self, not &'static str
    fn description(&self) -> &str { &self.description }
    fn schema(&self) -> serde_json::Value { self.schema_value.clone() }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let mut store = self.store.lock().await;
        let execute_fn = match self.instance
            .get_typed_func::<(i32, i32), i32>(&mut *store, "execute") {
            Ok(f) => f,
            Err(e) => return ToolResult { output: e.to_string(), is_error: true, metadata: None },
        };
        let args_json = match serde_json::to_string(&args) {
            Ok(s) => s,
            Err(e) => return ToolResult { output: e.to_string(), is_error: true, metadata: None },
        };
        let (ptr, len) = match write_wasm_string(&mut store, &args_json) {
            Ok(v) => v,
            Err(e) => return ToolResult { output: e.to_string(), is_error: true, metadata: None },
        };
        let result_ptr = match execute_fn.call(&mut *store, (ptr, len)) {
            Ok(p) => p,
            Err(e) => return ToolResult { output: e.to_string(), is_error: true, metadata: None },
        };
        let result = match read_wasm_string(&mut store, result_ptr) {
            Ok(s) => s,
            Err(e) => return ToolResult { output: e.to_string(), is_error: true, metadata: None },
        };
        serde_json::from_str(&result)
            .unwrap_or_else(|e| ToolResult { output: e.to_string(), is_error: true, metadata: None })
    }
}
```

Plugins implement a minimal ABI:
```wat
;; execute(args_ptr: i32, args_len: i32) -> result_ptr: i32
(func (export "execute") ...)
(func (export "schema") ...)
(func (export "description") ...)
```
---

## Related Documents

### Depends On
- [Cargo Workspace Design](12_Workspace_And_Crate_Structure.md)
- [Error Handling Strategy](../06_Concurrency/54_Error_Handling_Strategy.md)

### Used By
- [Core Agent Loop Design](13_Core_Agent_Loop_Design.md)
- [Tool Execution Engine](../04_Core_Features/30_Tool_Execution_Engine.md)
- [Terminal Tool](../04_Core_Features/30a_Terminal_Tool.md)
- [File System Tool](../04_Core_Features/31_File_System_Tool.md)
- [Browser Tool](../04_Core_Features/32_Browser_Tool.md)
- [Web Search Tool](../04_Core_Features/34_Web_Search_Tool.md)
- [MCP Client Tool](../04_Core_Features/36_MCP_Client_Tool.md)

### See Also
- [Approval Membrane](17a_Approval_Membrane.md)
- [Async Tool Execution](../06_Concurrency/50_Async_Tool_Execution.md)
- [Security Model](20_Security_Model.md)

