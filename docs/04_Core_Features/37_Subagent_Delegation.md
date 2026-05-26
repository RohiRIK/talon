# Subagent Delegation & Parallel Workstreams

> **Status:** ✅ Complete
> **Category:** Core Features
> **Last corrected:** dogfood pass 3

---

## 1. Architecture

Subagents are ephemeral Talon agent instances spawned as Tokio tasks.
They share no state with the parent beyond what is explicitly passed in the prompt.
Results are returned via `oneshot` channels.

```
Parent Agent Loop
      │
      │  delegate_task(tasks=[...])
      ▼
DelegationEngine
      ├── spawn task A → tokio::spawn(run_ephemeral_agent(A))
      ├── spawn task B → tokio::spawn(run_ephemeral_agent(B))
      └── spawn task C → tokio::spawn(run_ephemeral_agent(C))
             │
             ▼  (all run in parallel)
      JoinSet::join_next()
             │
             ▼
      [ResultA, ResultB, ResultC] → parent context
```

---

## 2. delegate_task Tool

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DelegateTaskParams {
    /// Single task mode
    pub goal: Option<String>,
    pub context: Option<String>,
    pub toolsets: Option<Vec<String>>,

    /// Batch mode (parallel)
    pub tasks: Option<Vec<SubtaskSpec>>,

    /// Role: "leaf" (default) or "orchestrator"
    #[serde(default = "default_role")]
    pub role: AgentRole,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SubtaskSpec {
    pub goal: String,
    pub context: Option<String>,
    pub toolsets: Option<Vec<String>>,
    pub role: Option<AgentRole>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Leaf,
    Orchestrator,
}

fn default_role() -> AgentRole { AgentRole::Leaf }
```

---

## 3. DelegationEngine

```rust
pub struct DelegationEngine {
    config: Arc<Config>,
    llm: Arc<dyn LlmProvider>,
    tools: Arc<ToolRegistry>,
    memory: Arc<MemoryStore>,
    max_concurrent: usize,
    max_depth: usize,
}

impl DelegationEngine {
    pub async fn delegate(
        &self,
        params: DelegateTaskParams,
        parent_ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        // Depth check — prevent infinite nesting
        let current_depth = parent_ctx.delegation_depth;
        if current_depth >= self.max_depth {
            return Err(ToolError::DelegationDepthExceeded {
                current: current_depth,
                max: self.max_depth,
            });
        }

        let tasks: Vec<SubtaskSpec> = match (params.goal, params.tasks) {
            (Some(goal), None) => vec![SubtaskSpec {
                goal,
                context: params.context,
                toolsets: params.toolsets,
                role: None,
            }],
            (None, Some(tasks)) => tasks,
            _ => return Err(ToolError::InvalidParams(
                "Provide either 'goal' or 'tasks', not both".into()
            )),
        };

        // Concurrency limit
        let tasks = tasks.into_iter().take(self.max_concurrent).collect::<Vec<_>>();

        let mut set = JoinSet::new();
        for task in tasks {
            let engine = self.clone_for_child();
            let depth = current_depth + 1;
            set.spawn(async move {
                engine.run_subagent(task, depth).await
            });
        }

        let mut results = vec![];
        while let Some(res) = set.join_next().await {
            match res {
                Ok(Ok(r)) => results.push(r),
                Ok(Err(e)) => results.push(SubagentResult {
                    status: "error".into(),
                    summary: e.to_string(),
                    tool_calls: 0,
                }),
                Err(e) => results.push(SubagentResult {
                    status: "panic".into(),
                    summary: e.to_string(),
                    tool_calls: 0,
                }),
            }
        }

        Ok(ToolResult::text(format_delegation_results(&results)))
    }

    async fn run_subagent(
        &self,
        task: SubtaskSpec,
        depth: usize,
    ) -> Result<SubagentResult, AgentError> {
        // Build isolated context
        let session_id = Uuid::new_v4();

        // Toolset filtering
        let tools = if let Some(ref toolsets) = task.toolsets {
            self.tools.filter_by_toolsets(toolsets)
        } else {
            self.tools.clone()
        };

        // Leaf agents cannot delegate further
        let tools = if matches!(task.role, Some(AgentRole::Leaf) | None) {
            tools.without("delegate_task")
        } else {
            tools
        };

        let memory = Arc::new(MemoryStore::in_memory().await?);

        let ctx = AgentContext {
            session_id,
            delegation_depth: depth,
            system_prompt: build_subagent_system_prompt(&task),
            ..AgentContext::default()
        };

        let mut agent = Agent::new(
            self.llm.clone(),
            tools,
            memory,
            self.config.clone(),
        );

        let result = agent.run_once(&task.goal, ctx).await?;

        Ok(SubagentResult {
            status: "completed".into(),
            summary: result.final_response,
            tool_calls: result.tool_call_count,
        })
    }
}
```

---

## 4. Toolset Filtering

Toolsets are named groups of tools — identical to Hermes:

```rust
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    toolsets: HashMap<String, Vec<String>>,
}

impl ToolRegistry {
    pub fn filter_by_toolsets(&self, names: &[String]) -> Arc<ToolRegistry> {
        let allowed: HashSet<&str> = names.iter()
            .flat_map(|name| {
                self.toolsets.get(name.as_str())
                    .map(|tools| tools.iter().map(String::as_str).collect::<Vec<_>>())
                    .unwrap_or_default()
            })
            .collect();

        Arc::new(ToolRegistry {
            tools: self.tools.iter()
                .filter(|(k, _)| allowed.contains(k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            toolsets: self.toolsets.clone(),
        })
    }
}
```

Default toolset definitions:

```toml
# config.toml
[toolsets]
web = ["web_search", "web_extract"]
browser = ["browser_navigate", "browser_snapshot", "browser_click", "browser_type", "browser_vision"]
terminal = ["terminal", "process"]
file = ["read_file", "write_file", "patch", "search_files"]
memory = ["memory", "session_search", "skill_view", "skills_list"]
delegation = ["delegate_task"]
```

---

## 5. Result Formatting

```rust
fn format_delegation_results(results: &[SubagentResult]) -> String {
    if results.len() == 1 {
        return results[0].summary.clone();
    }

    results.iter().enumerate().map(|(i, r)| {
        format!(
            "**Task {}** (status: {}, {} tool calls)\n\n{}",
            i + 1, r.status, r.tool_calls, r.summary
        )
    }).collect::<Vec<_>>().join("\n\n---\n\n")
}
```

---

## 6. Key Invariants

| Rule | Reason |
|------|--------|
| Leaf agents cannot call `delegate_task` | Prevents runaway recursive spawning |
| `max_concurrent` capped at config value (default: 3) | Prevents token/cost explosion |
| Each subagent gets its own `session_id` | Clean isolation, independent history |
| Subagents use in-memory SQLite | No cross-contamination with parent memory |
| Parent passes context explicitly | Subagents have no access to parent messages |
| Depth limit enforced before spawn | Hard cap, not soft guidance |
| Subagent failures don't abort sibling tasks | Partial results returned |
| No `clarify` in subagents | They run headlessly — can't ask the user questions |
---

## Related Documents

### Depends On
- [Subagent & Delegation Architecture](../02_Architecture/19_Subagent_And_Delegation_Architecture.md)
- [Core Agent Loop Design](../02_Architecture/13_Core_Agent_Loop_Design.md)

### See Also
- [Parallel Subagent Spawning](../06_Concurrency/51a_Parallel_Subagent_Spawning.md)
- [Channel Patterns](../06_Concurrency/51_Channel_Patterns.md)
- [Resource Limits & Backpressure](../06_Concurrency/53_Resource_Limits_And_Backpressure.md)

