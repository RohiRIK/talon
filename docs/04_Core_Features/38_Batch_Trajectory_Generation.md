# Batch Trajectory Generation

> **Status:** ✅ Complete
> **Category:** Core Features

---

## Overview

A **trajectory** is a complete recorded agent run: the initial task, every tool call made, every tool result received, intermediate reasoning steps, and the final response. In the context of the Hermes Agent [Self-Evolution](39_Self_Evolution_Loop.md) system, batch trajectory generation is the **data-collection phase** that feeds the GEPA evolutionary optimizer — not training data for fine-tuning a model.

> **Important:** Hermes Agent Self-Evolution explicitly requires **no GPU training**. Trajectories are used as evaluation inputs for DSPy-based LLM mutation and scoring, not as fine-tuning corpora. The entire loop runs on API calls, costing roughly **$2–10 per evolution run**.

---

## 1. What Trajectories Are Used For

Trajectories serve as **ground-truth evaluation data** for the GEPA optimizer. When a candidate skill, tool description, system prompt, or code variant is being evaluated, it is scored against a batch of execution traces:

- Did the agent succeed at the task when using this variant?
- How many tool calls did it take?
- Did it backtrack, error, or produce poor output?

This gives the evolutionary optimizer a **fitness signal** without needing a human in the loop. The four artifact types that can be evolved via this mechanism are:

1. **Skills** — SKILL.md procedural instruction files (Phase 1, currently implemented)
2. **Tool descriptions** — The natural-language descriptions passed to the LLM in the tools schema (Phase 2)
3. **System prompts** — The agent's core system prompt and persona (Phase 3)
4. **Code** — Actual Python tool implementation logic via the Darwinian Evolver (Phase 4)

---

## 2. Python Implementation: `batch_runner`

The trajectory collection system is implemented in Python, orchestrated via DSPy. The primary entry point for Phase 1 (Skill Evolution) is the `evolve_skill` module, which uses `batch_runner` to generate evaluation data.

```python
# batch_runner.py — simplified from hermes-agent-self-evolution

import dspy
import json
from dataclasses import dataclass, field
from typing import Optional
from pathlib import Path

@dataclass
class TrajectoryStep:
    role: str           # "user" | "assistant" | "tool_call" | "tool_result"
    content: str
    tool_name: Optional[str] = None
    tool_args: Optional[dict] = None
    is_error: bool = False

@dataclass
class Trajectory:
    task: str
    outcome: str        # "success" | "failure" | "truncated"
    steps: list[TrajectoryStep] = field(default_factory=list)
    skill_variant: Optional[str] = None   # which skill text was active
    total_tool_calls: int = 0
    duration_s: float = 0.0
    notes: str = ""

def run_task_with_skill(task: str, skill_text: str, agent_config: dict) -> Trajectory:
    """
    Run a single task against the agent with a specific skill variant active.
    Records all steps into a Trajectory for downstream fitness evaluation.
    """
    steps = [TrajectoryStep(role="user", content=task)]
    # ... agent invocation, step recording, outcome detection ...
    return Trajectory(task=task, outcome="success", steps=steps,
                      skill_variant=skill_text)

def run_batch(tasks: list[str], skill_text: str, agent_config: dict,
              concurrency: int = 3) -> list[Trajectory]:
    """
    Run a batch of evaluation tasks in parallel and collect trajectories.
    These are fed into the GEPA fitness evaluator.
    """
    from concurrent.futures import ThreadPoolExecutor
    with ThreadPoolExecutor(max_workers=concurrency) as ex:
        futures = [ex.submit(run_task_with_skill, t, skill_text, agent_config)
                   for t in tasks]
        return [f.result() for f in futures]
```

---

## 3. DSPy Integration: Trajectories as Evaluation Examples

DSPy treats trajectories as `dspy.Example` objects. The `batch_runner` output is transformed into a DSPy-compatible dataset, which GEPA then uses to score candidate artifact mutations:

```python
import dspy

def trajectories_to_dspy_examples(trajectories: list[Trajectory]) -> list[dspy.Example]:
    """
    Convert raw trajectories into DSPy Example objects for optimization.
    Each example provides an input (task) and a gold label (expected outcome).
    """
    examples = []
    for traj in trajectories:
        ex = dspy.Example(
            task=traj.task,
            expected_outcome="success",
            reference_steps=len(traj.steps),
        ).with_inputs("task")
        examples.append(ex)
    return examples

# DSPy metric: did the evolved skill help the agent succeed?
def skill_fitness_metric(example: dspy.Example, prediction, trace=None) -> float:
    outcome = prediction.get("outcome", "failure")
    tool_calls = prediction.get("total_tool_calls", 999)

    score = 1.0 if outcome == "success" else 0.0
    # Penalize excessive tool calls (efficiency signal)
    efficiency_bonus = max(0.0, 1.0 - (tool_calls / 20.0))
    return score * 0.8 + efficiency_bonus * 0.2
```

The GEPA optimizer uses this metric to rank population members across **multiple objectives** (success rate, efficiency, brevity), forming a Pareto frontier rather than collapsing to a single scalar.

---

## 4. Data Collected Per Trajectory

Each trajectory record captures the following fields, persisted as JSON:

| Field | Description |
|---|---|
| `task` | The input task string |
| `outcome` | `"success"`, `"failure"`, or `"truncated"` |
| `steps` | Ordered list of user/assistant/tool messages |
| `skill_variant` | Hash or text of the skill file active during this run |
| `total_tool_calls` | Count of tool invocations |
| `duration_s` | Wall-clock time for the run |
| `model` | LLM model used |
| `notes` | Reflective commentary from the LLM evaluator |

Trajectories are written to disk as `.jsonl` files in the `trajectories/` directory, one record per line.

---

## 5. Task File Format

Tasks are provided as plain-text files, one task per line:

```text
# eval_tasks.txt
Search the web for today's top AI news and summarize 3 headlines
Write a Python function to flatten a nested dictionary
List all files modified in the last 24 hours in the current directory
Create a reminder for tomorrow at 9am to review the weekly report
Explain what the skill 'web_search' does based on its SKILL.md file
```

The `evolve_skill` entry point accepts this file and the target skill name:

```bash
python -m evolve_skill \
  --skill-name web_search \
  --tasks eval_tasks.txt \
  --population-size 8 \
  --generations 5
```

---

## 6. Constraint Gates

Before any trajectory is used to promote a mutated artifact, it passes through **constraint gates** — validation checks that prevent regressions:

- **Syntax check:** Does the mutated skill parse as valid Markdown with correct YAML front matter?
- **Safety check:** Does the LLM-based judge flag any harmful or policy-violating content?
- **Regression gate:** Does the candidate perform at least as well as the current champion on a held-out reference set?

Only candidates that clear all gates enter the Pareto population for further evolution.

---

## 7. Talon (Rust) Implementation

Talon will implement batch trajectory generation natively, without Python subprocess calls. The core data model maps closely:

```rust
// talon-core/src/evolution/trajectory.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trajectory {
    pub id: Uuid,
    pub task: String,
    pub outcome: TrajectoryOutcome,
    pub steps: Vec<TrajectoryStep>,
    pub skill_variant_hash: Option<String>,
    pub total_tool_calls: usize,
    pub duration_ms: u64,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrajectoryOutcome {
    Success,
    Failure { error: String },
    Truncated { reason: String },
}
```

Batch execution will use `tokio::task::JoinSet` with a semaphore for concurrency control, writing results to both SQLite (for indexed querying) and `.jsonl` (for compatibility with the Python GEPA optimizer when running the external evolution pipeline):

```rust
pub async fn run_batch(
    tasks: Vec<String>,
    skill_variant: Option<String>,
    concurrency: usize,
) -> Vec<Trajectory> {
    let sem = Arc::new(Semaphore::new(concurrency));
    let mut set = JoinSet::new();

    for task in tasks {
        let permit = sem.clone().acquire_owned().await.unwrap();
        let variant = skill_variant.clone();
        set.spawn(async move {
            let _p = permit;
            run_single_task(task, variant).await
        });
    }

    let mut results = Vec::new();
    while let Some(Ok(traj)) = set.join_next().await {
        results.push(traj);
    }
    results
}
```

The key difference from the Python system is that Talon's batch runner is **embedded** — it generates trajectories from its own live agent loop rather than shelling out to a separate process.

---

## See Also

- `39_Self_Evolution_Loop.md` — How GEPA uses these trajectories to evolve artifacts
- `40_Skill_System.md` — Skill file format that is the primary evolution target (Phase 1)
---

## Related Documents

### Depends On
- [Core Agent Loop Design](../02_Architecture/13_Core_Agent_Loop_Design.md)

### See Also
- [Self-Evolution Loop](39_Self_Evolution_Loop.md)
- [Subagent Delegation](37_Subagent_Delegation.md)

