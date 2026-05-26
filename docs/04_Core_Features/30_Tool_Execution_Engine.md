# Tool Execution Engine

> **Last corrected:** dogfood pass 2
>
 > **Status:** ✅ Complete
> **Category:** Core Features

---

## 1. What is the Tool Execution Engine?

The Tool Execution Engine (TEE) is the component that:
1. Receives `ToolCall` requests from the LLM
2. Validates and deserializes arguments
3. Routes to the correct `Tool` implementation
4. Applies approval checks (risk level gating)
5. Executes and captures output
6. Returns `ToolResult` to the LLM

---

## 2. ToolCall Lifecycle

```
LLM Response
   │
   ▼
[Parse tool_use blocks]
   │
   ▼
[Schema Validation] ──fail──► ToolResult::schema_error()
   │
   ▼
[Risk Assessment] ──high──► [Approval Gate] ──denied──► ToolResult::denied()
   │                                │
   │                            approved
   │◄──────────────────────────────┘
   ▼
[Tool::execute(args, ctx)]
   │
   ▼
[Output Limiting] (50KB max)
   │
   ▼
ToolResult → LLM
```

---

## 3. Core Types

```rust
// talon-core/src/tools/mod.rs

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> serde_json::Value;         // JSON Schema for args
    fn approval_level(&self) -> ApprovalLevel {    // default: Safe
        ApprovalLevel::Safe
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> ToolResult;
}

pub struct ToolContext {
    pub session_id: Uuid,
    pub workdir: Option<PathBuf>,
    pub profile_dir: PathBuf,
    pub approval_tx: Option<mpsc::Sender<ApprovalRequest>>,
}

pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
    pub metadata: Option<serde_json::Value>,
}

impl ToolResult {
    pub fn success(output: impl Into<String>) -> Self {
        Self { output: output.into(), is_error: false, metadata: None }
    }
    pub fn error(output: impl Into<String>) -> Self {
        Self { output: output.into(), is_error: true, metadata: None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ApprovalLevel {
    Safe,          // Always execute
    Confirmation,  // Ask once, remember answer
    Required,      // Always ask, no memory
    Blocked,       // Never execute
}
```

---

## 4. Tool Registry

```rust
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, Arc::from(tool));
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn all_schemas(&self) -> Vec<serde_json::Value> {
        self.tools.values().map(|t| {
            json!({
                "name": t.name(),
                "description": t.description(),
                "input_schema": t.schema()
            })
        }).collect()
    }
}
```

---

## 5. Execution & Error Handling

```rust
pub struct ToolExecutor {
    registry: Arc<ToolRegistry>,
    approval_service: Arc<ApprovalService>,
    config: ToolsConfig,
}

impl ToolExecutor {
    pub async fn execute_call(
        &self,
        call: &ToolCall,
        ctx: &ToolContext,
    ) -> ToolResult {
        // 1. Lookup tool
        let tool = match self.registry.get(&call.name) {
            Some(t) => t,
            None => return ToolResult::error(format!(
                "Unknown tool: '{}'. Available: {}",
                call.name,
                self.registry.tool_names().join(", ")
            )),
        };

        // 2. Schema validation
        if let Err(e) = self.validate_args(tool.as_ref(), &call.args) {
            return ToolResult::error(format!("Invalid arguments: {e}"));
        }

        // 3. Approval check
        if tool.approval_level() >= ApprovalLevel::Confirmation {
            match self.approval_service.request_approval(call, ctx).await {
                Ok(ApprovalDecision::Approved) => {}
                Ok(ApprovalDecision::Denied) => {
                    return ToolResult::error(format!(
                        "Tool '{}' was denied by user", call.name
                    ));
                }
                Err(e) => {
                    return ToolResult::error(format!("Approval request failed: {e}"));
                }
            }
        }

        // 4. Execute with timeout
        let timeout = self.config.timeout_for(&call.name);
        match tokio::time::timeout(timeout, tool.execute(call.args.clone(), ctx)).await {
            Ok(result) => {
                // 5. Apply output size limit
                self.limit_output(result)
            }
            Err(_) => ToolResult::error(format!(
                "Tool '{}' timed out after {}s", call.name, timeout.as_secs()
            )),
        }
    }

    fn limit_output(&self, mut result: ToolResult) -> ToolResult {
        const MAX_BYTES: usize = 50_000;
        if result.output.len() > MAX_BYTES {
            result.output.truncate(MAX_BYTES);
            result.output.push_str("\n[Output truncated — exceeded 50KB limit]");
        }
        result
    }
}
```

---

## 6. Parallel Tool Execution

When the LLM returns multiple tool calls in one response, Talon
executes them in parallel if they're all `[ApprovalLevel](../02_Architecture/17a_Approval_Membrane.md)::Safe`:

```rust
pub async fn execute_parallel(
    &self,
    calls: Vec<ToolCall>,
    ctx: &ToolContext,
) -> Vec<(ToolCall, ToolResult)> {
    // Separate by risk: safe → parallel, others → sequential
    let (safe, gated): (Vec<_>, Vec<_>) = calls.into_iter().partition(|c| {
        self.registry.get(&c.name)
            .map(|t| t.approval_level() == ApprovalLevel::Safe)
            .unwrap_or(false)
    });

    let mut results = vec![];

    // Safe tools: parallel
    let safe_futures: Vec<_> = safe.into_iter().map(|call| {
        let executor = self.clone();
        let ctx = ctx.clone();
        async move {
            let result = executor.execute_call(&call, &ctx).await;
            (call, result)
        }
    }).collect();
    results.extend(futures::future::join_all(safe_futures).await);

    // Gated tools: sequential (each may need user interaction)
    for call in gated {
        let result = self.execute_call(&call, ctx).await;
        results.push((call, result));
    }

    results
}
```
---

## Related Documents

### Depends On
- [Tool System Architecture](../02_Architecture/16_Tool_System_Architecture.md)
- [Async Tool Execution](../06_Concurrency/50_Async_Tool_Execution.md)

### Used By
- [Agent Loop Implementation](29_Agent_Loop_Implementation.md)

### See Also
- [Error Handling Strategy](../06_Concurrency/54_Error_Handling_Strategy.md)
- [Channel Patterns](../06_Concurrency/51_Channel_Patterns.md)

