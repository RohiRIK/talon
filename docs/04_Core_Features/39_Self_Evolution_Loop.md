# Self-Evolution Loop

> **Status:** ✅ Complete
> **Category:** Core Features

---

## Overview

The Hermes Agent Self-Evolution system uses **GEPA (Genetic-Pareto Prompt Evolution)** combined with **DSPy** to automatically improve Hermes Agent's internal artifacts through evolutionary search — **without any GPU training or fine-tuning**. It operates as a standalone Python pipeline that runs against a live agent, costs roughly **$2–10 per optimization run**, and requires only API access.

This document covers:
1. How GEPA works as an algorithm
2. The 5-phase optimization roadmap (Phase 1 currently implemented)
3. The 4 artifact types targeted for evolution
4. How Talon (Rust) would natively implement equivalent functionality

---

## 1. GEPA: Genetic-Pareto Prompt Evolution

GEPA is an evolutionary optimization algorithm that operates on **text artifacts** (skills, prompts, descriptions, code). Unlike gradient-based optimization, GEPA uses LLM calls as mutation and crossover operators — no gradients, no GPU.

### Core Loop

```
Initialize population of N candidate artifacts
Evaluate each candidate against a batch of tasks → fitness scores
Select Pareto-optimal members (multi-objective: success rate + efficiency + brevity)
Apply mutation operators via LLM to create offspring
Run offspring through constraint gates
Replace dominated members with superior offspring
Repeat for G generations
Champion = highest-scoring member of final Pareto front
```

### Why Pareto, Not Single-Objective?

A single fitness score (e.g., "success rate") produces over-specialized artifacts. GEPA instead maintains a **Pareto frontier** across multiple objectives simultaneously:

- **Effectiveness** — does the agent succeed at the task?
- **Efficiency** — how many tool calls / tokens does it use?
- **Compactness** — is the artifact concise and maintainable?

A candidate survives if no other candidate dominates it on *all* objectives at once. This preserves diversity in the population and prevents convergence to brittle local optima.

---

## 2. Python + DSPy Implementation

The system is implemented in Python. DSPy provides the LLM abstraction layer; GEPA is the evolutionary search strategy built on top of it.

### Skill Evolution Entry Point (Phase 1)

```python
# evolve_skill.py — primary entry point for Phase 1

import dspy
from gepa import GEPAOptimizer
from batch_runner import run_batch, trajectories_to_dspy_examples
from constraint_gates import validate_candidate

def evolve_skill(
    skill_name: str,
    tasks_file: str,
    population_size: int = 8,
    generations: int = 5,
) -> str:
    """
    Evolve a Hermes Agent SKILL.md file using GEPA.
    Returns the champion skill text after evolution.
    """
    # 1. Load current skill text as seed for population
    skill_path = find_skill_path(skill_name)
    seed_text = skill_path.read_text()

    # 2. Load evaluation tasks
    tasks = [t.strip() for t in open(tasks_file) if t.strip() and not t.startswith("#")]

    # 3. Configure DSPy LM
    lm = dspy.LM("openai/gpt-4o", max_tokens=4096)
    dspy.configure(lm=lm)

    # 4. Run GEPA optimization
    optimizer = GEPAOptimizer(
        population_size=population_size,
        generations=generations,
        mutation_fn=llm_mutate_skill,
        fitness_fn=lambda variant: evaluate_skill(variant, tasks),
        constraint_fn=validate_candidate,
    )

    champion = optimizer.run(seed=seed_text)
    return champion
```

### Population Management

```python
# gepa.py — simplified GEPA core

from dataclasses import dataclass
from typing import Callable

@dataclass
class Individual:
    artifact: str           # the text being evolved
    fitness: dict           # {"success_rate": 0.8, "efficiency": 0.6, "compactness": 0.9}
    generation: int

def pareto_dominates(a: Individual, b: Individual) -> bool:
    """Returns True if a dominates b on ALL objectives."""
    objs = list(a.fitness.keys())
    return all(a.fitness[o] >= b.fitness[o] for o in objs) and \
           any(a.fitness[o] >  b.fitness[o] for o in objs)

def pareto_front(population: list[Individual]) -> list[Individual]:
    """Return only the non-dominated members of the population."""
    front = []
    for candidate in population:
        if not any(pareto_dominates(other, candidate)
                   for other in population if other is not candidate):
            front.append(candidate)
    return front

class GEPAOptimizer:
    def __init__(self, population_size, generations,
                 mutation_fn, fitness_fn, constraint_fn):
        self.pop_size = population_size
        self.generations = generations
        self.mutate = mutation_fn
        self.evaluate = fitness_fn
        self.validate = constraint_fn

    def run(self, seed: str) -> str:
        # Seed population with variations of the initial artifact
        population = self._seed_population(seed)

        for gen in range(self.generations):
            # Evaluate all members
            for ind in population:
                ind.fitness = self.evaluate(ind.artifact)

            # Select Pareto front as survivors
            survivors = pareto_front(population)

            # Mutate survivors to fill population back to pop_size
            offspring = []
            while len(offspring) < self.pop_size - len(survivors):
                parent = random.choice(survivors)
                child_text = self.mutate(parent.artifact)
                if self.validate(child_text):
                    offspring.append(Individual(child_text, {}, gen + 1))

            population = survivors + offspring

        # Return the champion: highest success_rate on final Pareto front
        final_front = pareto_front(population)
        return max(final_front, key=lambda i: i.fitness.get("success_rate", 0)).artifact
```

### LLM Mutation Operators

GEPA uses DSPy `Signature`-based modules as mutation operators. The LLM itself rewrites the artifact based on reflective analysis:

```python
import dspy

class MutateSkill(dspy.Signature):
    """Rewrite a Hermes Agent skill file to improve agent performance.
    The new version should be clearer, more actionable, and fix observed failure modes."""
    current_skill: str = dspy.InputField(desc="The current SKILL.md text")
    failure_examples: str = dspy.InputField(desc="Tasks where the agent failed with this skill")
    improved_skill: str = dspy.OutputField(desc="The rewritten SKILL.md text")

class CrossoverSkills(dspy.Signature):
    """Combine the best elements of two skill variants into a single improved version."""
    skill_a: str = dspy.InputField()
    skill_b: str = dspy.InputField()
    merged_skill: str = dspy.OutputField()

def llm_mutate_skill(skill_text: str, failures: list[str] = None) -> str:
    mutator = dspy.Predict(MutateSkill)
    result = mutator(
        current_skill=skill_text,
        failure_examples="\n".join(failures or [])
    )
    return result.improved_skill
```

---

## 3. The 5-Phase Optimization Roadmap

The system targets four artifact types across five implementation phases:

| Phase | Artifact | Optimizer | Status |
|---|---|---|---|
| 1 | Skills (SKILL.md files) | GEPA | ✅ Implemented |
| 2 | Tool descriptions | GEPA | 🔲 Planned |
| 3 | System prompts | MIPROv2 + GEPA | 🔲 Planned |
| 4 | Tool implementation code | Darwinian Evolver | 🔲 Planned |
| 5 | End-to-end multi-artifact | Combined pipeline | 🔲 Planned |

### Phase 4: Darwinian Evolver (Code Evolution)

Phase 4 extends GEPA to evolve actual Python code. Each population member is a **`GitBasedOrganism`** — a specific commit or branch of the codebase. The fitness function runs the agent's test suite against that commit and scores it on pass rate, performance, and error frequency.

---

## 4. Constraint Gates

Every candidate mutation must pass **constraint gates** before entering the population:

```python
# constraint_gates.py

def validate_candidate(artifact: str, artifact_type: str = "skill") -> bool:
    """Gate 1: Structural validity (parses correctly)."""
    if artifact_type == "skill":
        return is_valid_skill_markdown(artifact)

def regression_gate(candidate: str, champion: str, ref_tasks: list[str]) -> bool:
    """Gate 2: Candidate must not regress vs. current champion on reference set."""
    candidate_score = quick_eval(candidate, ref_tasks)
    champion_score = quick_eval(champion, ref_tasks)
    return candidate_score >= champion_score * 0.95  # Allow 5% tolerance

def safety_gate(artifact: str) -> bool:
    """Gate 3: LLM judge checks for harmful or policy-violating content."""
    judge = dspy.Predict(SafetyJudge)
    result = judge(artifact=artifact)
    return result.is_safe
```

---

## 5. Talon (Rust) Implementation

Talon would implement equivalent GEPA evolution natively in Rust, orchestrating the same loop without Python subprocess calls. The architecture maps naturally:

```rust
// talon-core/src/evolution/gepa.rs

pub struct Individual {
    pub artifact: String,
    pub fitness: HashMap<String, f64>,
    pub generation: usize,
}

pub fn pareto_dominates(a: &Individual, b: &Individual) -> bool {
    let all_gte = a.fitness.iter().all(|(k, v)| b.fitness.get(k).map_or(false, |bv| v >= bv));
    let any_gt  = a.fitness.iter().any(|(k, v)| b.fitness.get(k).map_or(false, |bv| v > bv));
    all_gte && any_gt
}

pub fn pareto_front(population: &[Individual]) -> Vec<&Individual> {
    population.iter().filter(|candidate| {
        !population.iter().any(|other| {
            !std::ptr::eq(other, *candidate) && pareto_dominates(other, candidate)
        })
    }).collect()
}

pub struct GEPAEvolver {
    pub population_size: usize,
    pub generations: usize,
    pub llm: Arc<dyn LlmProvider>,
    pub evaluator: Arc<dyn ArtifactEvaluator>,
}

impl GEPAEvolver {
    pub async fn evolve(&self, seed: String) -> anyhow::Result<String> {
        let mut population = self.seed_population(&seed).await?;

        for gen in 0..self.generations {
            // Evaluate all in parallel
            let mut evaluated = vec![];
            for mut ind in population {
                ind.fitness = self.evaluator.evaluate(&ind.artifact).await?;
                evaluated.push(ind);
            }

            // Pareto selection
            let survivors: Vec<Individual> = pareto_front(&evaluated)
                .into_iter().cloned().collect();

            // Mutate to refill population
            let mut offspring = vec![];
            while survivors.len() + offspring.len() < self.population_size {
                let parent = &survivors[rand::random::<usize>() % survivors.len()];
                let child_text = self.llm_mutate(&parent.artifact).await?;
                if self.validate_candidate(&child_text) {
                    offspring.push(Individual {
                        artifact: child_text,
                        fitness: HashMap::new(),
                        generation: gen + 1,
                    });
                }
            }

            population = [survivors, offspring].concat();
            tracing::info!(gen, pop = population.len(), "GEPA generation complete");
        }

        // Champion = best success_rate on final Pareto front
        let final_front = pareto_front(&population);
        let champion = final_front.iter()
            .max_by(|a, b| a.fitness.get("success_rate")
                .partial_cmp(&b.fitness.get("success_rate"))
                .unwrap_or(std::cmp::Ordering::Equal))
            .ok_or_else(|| anyhow::anyhow!("Empty population"))?;

        Ok(champion.artifact.clone())
    }
}
```

### Key Design Difference: No GPU, No Fine-tuning

Talon's evolution loop — like the Python reference implementation — relies exclusively on **LLM API calls** for mutation, evaluation, and scoring. There is no gradient computation, no weight update, and no local model training. The "learning" happens at the **artifact level** (text files) not at the **weight level** (model parameters).

This means evolution runs on any machine with internet access and an API key, and the cost scales with the number of LLM calls per generation, not with hardware.

---

## 6. Running the Python Evolution Pipeline Against Talon

Since Phase 1 is already implemented in the Python repo, Talon deployments can invoke it externally today:

```bash
# Install the evolution pipeline
git clone https://github.com/NousResearch/hermes-agent-self-evolution
cd hermes-agent-self-evolution
pip install -e .

# Point it at Talon's skill directory and run evolution on a target skill
python -m evolve_skill \
  --skill-name web_search \
  --skills-dir ~/.talon/skills/ \
  --tasks eval_tasks.txt \
  --population-size 8 \
  --generations 5 \
  --output evolved_skill.md

# Review the champion before deploying
diff ~/.talon/skills/web_search.md evolved_skill.md

# Promote if satisfied
cp evolved_skill.md ~/.talon/skills/web_search.md
```

---

## See Also

- `38_Batch_Trajectory_Generation.md` — How execution traces are collected for fitness evaluation
- `40_Skill_System.md` — Skill file format (the primary target of Phase 1 evolution)
- [hermes-agent-self-evolution on GitHub](https://github.com/NousResearch/hermes-agent-self-evolution) — Reference Python implementation
---

## Related Documents

### Depends On
- [Plugin & Skill Architecture](../02_Architecture/17_Plugin_And_Skill_Architecture.md)
- [Core Agent Loop Design](../02_Architecture/13_Core_Agent_Loop_Design.md)

### See Also
- [Skill Store](../07_Memory_System/57_Skill_Store.md)
- [Batch Trajectory Generation](38_Batch_Trajectory_Generation.md)
- [Skill System](34a_Skill_System.md)

