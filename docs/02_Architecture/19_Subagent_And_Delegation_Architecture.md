# Subagent & Delegation Architecture

> **Status:** ✅ Complete
> **Category:** Architecture
> **Last corrected:** dogfood pass 3

---

## 1. Design Overview

Talon supports spawning isolated sub-agents that work on tasks in parallel.
Each subagent gets:
- Its own Tokio task (or process for full isolation)
- Its own context/message history
- Its own tool registry (can be restricted)
- Its own LLM provider/model (can differ from parent)

Delegation is the core primitive that enables autonomous multi-step work.

---

## 2. Delegation Modes

```
delegate_task(goal, context)       ← single subagent
delegate_task(tasks: [...])        ← batch (up to N parallel)

SubagentMode:
  ├── InProcess    → Tokio task, shares process, fast
  └── Subprocess   → OS process, full isolation, sandboxed
```

**In-process mode** (default): same binary, `tokio::spawn`, shared memory
allocator. Fast, zero startup overhead. Used for most delegations.

**Subprocess mode** (optional): spawns a new `talon` process with `--subagent`
flag. Full memory and filesystem isolation. Used when:
- Tool execution needs a clean environment
- Subagent might crash (don't want to take down parent)
- Security-critical operations

---

## 3. Core Types

```rust
// talon-core/src/delegation.rs

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DelegateTaskParams {
    /// What the subagent should accomplish (single-task mode)
    pub goal: Option<String>,
    /// Background context injected before goal
    pub context: Option<String>,
    /// Restrict toolsets available to subagent
    pub toolsets: Option<Vec<String>>,
    /// Batch mode: up to N parallel tasks
    pub tasks: Option<Vec<SubTask>>,
    /// Subagent role: leaf (default) or orchestrator
    #[serde(default)]
    pub role: SubagentRole,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SubTask {
    pub goal: String,
    pub context: Option<String>,
    pub toolsets: Option<Vec<String>>,
    pub role: Option<SubagentRole>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum SubagentRole {
    #[default]
    Leaf,         // cannot call delegate_task (prevents unbounded recursion)
    Orchestrator, // can spawn its own children (bounded by max_depth config)
}
```

---

## 4. Subagent Executor

```rust
pub struct SubagentExecutor {
    config: Arc<TalonConfig>,
    llm: Arc<dyn LlmProvider>,
    tool_registry: Arc<ToolRegistry>,
    memory: Arc<MemoryStore>,
    max_depth: u8,
}

impl SubagentExecutor {
    pub async fn run_batch(
        &self,
        tasks: Vec<SubTask>,
        parent_depth: u8,
    ) -> Vec<SubagentResult> {
        // Check recursion depth
        if parent_depth >= self.max_depth {
            return tasks.iter().map(|t| SubagentResult::Error(
                format!("Max delegation depth ({}) exceeded", self.max_depth)
            )).collect();
        }

        // Cap concurrency
        let max_concurrent = self.config.agent.max_concurrent_children;
        let semaphore = Arc::new(Semaphore::new(max_concurrent));

        let mut handles = JoinSet::new();

        for task in tasks {
            let sem = semaphore.clone();
            let executor = self.child_executor(parent_depth + 1);

            handles.spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                executor.run_single(task).await
            });
        }

        let mut results = vec![];
        while let Some(result) = handles.join_next().await {
            match result {
                Ok(r) => results.push(r),
                Err(e) => results.push(SubagentResult::Error(
                    format!("Subagent panicked: {e}")
                )),
            }
        }
        results
    }

    async fn run_single(&self, task: SubTask) -> SubagentResult {
        // Build isolated context
        let mut messages = vec![];

        if let Some(ctx) = &task.context {
            messages.push(Message::user(format!(
                "<context>\n{}\n</context>", ctx
            )));
        }

        messages.push(Message::user(task.goal.clone()));

        // Build restricted tool registry
        let registry = match &task.toolsets {
            Some(toolsets) => self.tool_registry.filtered(toolsets),
            None => self.tool_registry.without_delegate(), // leaves can't delegate
        };

        // Run agent loop in isolation
        let mut agent = AgentLoop::new(
            self.llm.clone(),
            registry,
            self.memory.clone(),
            messages,
            AgentConfig {
                max_iterations: 30,
                ..self.config.agent.clone()
            },
        );

        match agent.run().await {
            Ok(output) => SubagentResult::Success {
                summary: output.final_response,
                tool_calls_made: output.tool_call_count,
            },
            Err(e) => SubagentResult::Error(e.to_string()),
        }
    }

    fn child_executor(&self, depth: u8) -> Arc<SubagentExecutor> {
        // Leaf subagents get a registry without delegate_task
        let registry = if depth >= self.max_depth - 1 {
            self.tool_registry.without_delegate()
        } else {
            self.tool_registry.clone()
        };

        Arc::new(SubagentExecutor {
            config: self.config.clone(),
            llm: self.llm.clone(),
            tool_registry: registry,
            memory: self.memory.clone(),
            max_depth: self.max_depth,
        })
    }
}
```

---

## 5. The delegate_task Tool

```rust
pub struct DelegateTaskTool {
    executor: Arc<SubagentExecutor>,
    current_depth: u8,
}

#[async_trait]
impl Tool for DelegateTaskTool {
    fn name(&self) -> &str { "delegate_task" }
    fn risk_level(&self) -> ToolRisk { ToolRisk::Moderate }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let p: DelegateTaskParams = serde_json::from_value(args)?;

        let tasks: Vec<SubTask> = match (p.tasks, p.goal) {
            (Some(tasks), _) => tasks,
            (None, Some(goal)) => vec![SubTask {
                goal,
                context: p.context,
                toolsets: p.toolsets,
                role: Some(p.role),
            }],
            (None, None) => return Err(ToolError::InvalidParams(
                "Provide either 'goal' or 'tasks'".into()
            )),
        };

        let results = self.executor
            .run_batch(tasks, self.current_depth)
            .await;

        let output = results.iter().enumerate().map(|(i, r)| {
            match r {
                SubagentResult::Success { summary, .. } =>
                    format!("**Task {}:** {}", i + 1, summary),
                SubagentResult::Error(e) =>
                    format!("**Task {} (failed):** {}", i + 1, e),
            }
        }).collect::<Vec<_>>().join("\n\n");

        Ok(ToolResult::text(output))
    }
}
```

---

## 6. Depth Limiting & Cycle Prevention

```
Parent (depth=0)
  └── Orchestrator subagent (depth=1)
        ├── Leaf subagent A (depth=2) — cannot delegate further
        └── Leaf subagent B (depth=2) — cannot delegate further

Config: max_spawn_depth = 2 (default)
```

Enforcement: `DelegateTaskTool` is excluded from the registry when
`current_depth >= max_spawn_depth`. No runtime panic — just missing tool.

```rust
impl ToolRegistry {
    pub fn without_delegate(&self) -> Arc<ToolRegistry> {
        let mut registry = self.clone();
        registry.remove("delegate_task");
        Arc::new(registry)
    }

    pub fn for_depth(&self, depth: u8, max_depth: u8) -> Arc<ToolRegistry> {
        if depth >= max_depth {
            self.without_delegate()
        } else {
            Arc::new(self.clone())
        }
    }
}
```

---

## 7. Result Aggregation

When tasks run in parallel, results are returned in completion order:

```rust
#[derive(Debug)]
pub enum SubagentResult {
    Success {
        summary: String,
        tool_calls_made: usize,
    },
    Error(String),
    Interrupted,  // parent cancelled via CancellationToken
}
```

The parent agent sees all results as a single `ToolResult`. It can reason
about partial failures — if 2 of 3 tasks succeed, the parent decides
whether to retry the failed one or proceed.

---

## 8. Cancellation

Subagents are tied to a `CancellationToken`. When the parent is cancelled
(user sends `/stop`, session ends), all children are cancelled too:

```rust
pub struct AgentLoop {
    cancel: CancellationToken,
    // ...
}

// In run_batch:
let child_cancel = parent_cancel.child_token();
handles.spawn(async move {
    tokio::select! {
        result = executor.run_single(task) => result,
        _ = child_cancel.cancelled() => SubagentResult::Interrupted,
    }
});
```
---

## Related Documents

### Depends On
- [Core Agent Loop Design](13_Core_Agent_Loop_Design.md)
- [Tokio Runtime Design](../06_Concurrency/49_Tokio_Runtime_Design.md)

### Used By
- [Subagent Delegation](../04_Core_Features/37_Subagent_Delegation.md)

### See Also
- [Channel Patterns](../06_Concurrency/51_Channel_Patterns.md)
- [Parallel Subagent Spawning](../06_Concurrency/51a_Parallel_Subagent_Spawning.md)
- [Resource Limits & Backpressure](../06_Concurrency/53_Resource_Limits_And_Backpressure.md)

