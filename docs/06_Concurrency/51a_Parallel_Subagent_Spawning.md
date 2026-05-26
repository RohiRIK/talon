# Parallel Subagent Spawning

> **Status:** ✅ Complete
> **Category:** Concurrency

---

## 1. What Are Subagents?

Subagents are isolated Talon instances spawned for parallel workstreams.
The primary agent delegates a task to a subagent, waits for its result,
and incorporates it into the main conversation.

Use cases:
- Research task A and B simultaneously
- Run multiple code reviews in parallel
- Batch-process N items concurrently

---

## 2. Spawn Mechanism

Subagents are spawned as separate OS processes (not Tokio tasks).
This provides:
- Full isolation (no shared memory)
- Independent context windows
- Can use different models
- Crash in one doesn't affect the other

```rust
pub struct SubagentSpawner {
    talon_binary: PathBuf,
    max_concurrent: u32,
    semaphore: Arc<Semaphore>,
}

impl SubagentSpawner {
    pub async fn spawn(
        &self,
        goal: &str,
        context: Option<&str>,
        toolsets: &[&str],
    ) -> Result<String, SubagentError> {
        // Acquire spawn slot
        let _permit = self.semaphore.acquire().await
            .map_err(|_| SubagentError::SpawnLimitReached)?;

        let mut cmd = tokio::process::Command::new(&self.talon_binary);
        cmd.arg("--subagent")
           .arg("--goal").arg(goal);

        if let Some(ctx) = context {
            cmd.arg("--context").arg(ctx);
        }

        for toolset in toolsets {
            cmd.arg("--toolset").arg(toolset);
        }

        let output = cmd.output().await?;

        if output.status.success() {
            String::from_utf8(output.stdout)
                .map_err(|_| SubagentError::InvalidOutput)
        } else {
            let err = String::from_utf8_lossy(&output.stderr);
            Err(SubagentError::Failed(err.to_string()))
        }
    }
}
```

---

## 3. Parallel Batch Execution

For the `tasks` parameter in `[delegate_task](../04_Core_Features/37_Subagent_Delegation.md)`:

```rust
pub async fn run_parallel_tasks(
    spawner: &SubagentSpawner,
    tasks: Vec<SubagentTask>,
) -> Vec<SubagentResult> {
    let futures: Vec<_> = tasks.into_iter().map(|task| {
        let spawner = spawner.clone();
        async move {
            let result = spawner.spawn(
                &task.goal,
                task.context.as_deref(),
                &task.toolsets.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            ).await;

            SubagentResult {
                goal: task.goal,
                output: result.unwrap_or_else(|e| format!("[Error: {e}]")),
            }
        }
    }).collect();

    // All run concurrently, bounded by semaphore
    futures::future::join_all(futures).await
}
```

---

## 4. Result Injection

The orchestrator agent receives subagent outputs as tool results:

```
[delegate_task called with 2 parallel tasks]

Subagent 1 result: "React 19 concurrent features include..."
Subagent 2 result: "Rust async ecosystem in 2024..."

[LLM synthesizes both results into final response]
```

---

## 5. Spawn Depth Limits

To prevent infinite recursion (subagent spawning subagents):

```toml
[tools.delegate_task]
max_spawn_depth = 1    # subagents cannot spawn further subagents
max_concurrent = 3     # max parallel subagents
```

```rust
// Each subagent process receives its depth via env var
std::env::set_var("TALON_SPAWN_DEPTH", (ctx.spawn_depth + 1).to_string());

// At startup, read and enforce
let depth = std::env::var("TALON_SPAWN_DEPTH")
    .ok().and_then(|s| s.parse().ok()).unwrap_or(0);

if depth >= config.max_spawn_depth {
    // Disable delegate_task tool for this process
    tool_registry.disable("delegate_task");
}
```
---

## Related Documents

### Depends On
- [Subagent & Delegation Architecture](../02_Architecture/19_Subagent_And_Delegation_Architecture.md)
- [Channel Patterns](51_Channel_Patterns.md)

### See Also
- [Subagent Delegation](../04_Core_Features/37_Subagent_Delegation.md)
- [Resource Limits & Backpressure](53_Resource_Limits_And_Backpressure.md)

