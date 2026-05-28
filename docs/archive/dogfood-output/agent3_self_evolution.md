# Agent 3 — Self-Evolution Repo Audit

> **Source audited:** https://github.com/NousResearch/hermes-agent-self-evolution
> **Docs checked:**
> - `/home/rohi/homelab/projects/ernest/docs/04_Core_Features/39_Self_Evolution_Loop.md`
> - `/home/rohi/homelab/projects/ernest/docs/04_Core_Features/38_Batch_Trajectory_Generation.md`
> - *(Note: `01_Analysis/07_Self_Evolution_Analysis.md` does not exist — likely not yet written)*

---

## Confirmed Accurate

The following claims in our Ernest docs are **consistent with** the actual Hermes Agent Self-Evolution repo:

- **Skills as the primary evolution target** — The repo's Phase 1 (implemented) does evolve skill `.md` files, matching Ernest's skill extraction pipeline.
- **Execution traces / agent runs as input** — The repo uses execution traces (agent run outputs) as the raw material for optimization, matching the trajectory-collection concept in our `38_Batch_Trajectory_Generation.md`.
- **LLM-driven analysis** — Both our docs and the repo use LLMs to analyze runs and generate improved variants (prompts/skills).
- **JSONL as a data format** — The PLAN.md references `.jsonl` for iterations; our batch runner also exports JSONL.
- **Iterative refinement loop** — Both describe a loop: run → evaluate → improve → repeat.
- **Skill promotion/discard based on performance** — Both have a fitness/pass-rate gate before a candidate skill is promoted.

---

## Inaccuracies

These are places where our Ernest docs describe something **differently** from how the actual Hermes repo works:

### 1. **Technology stack is wrong**
- **Our docs claim:** Rust implementation (`EvolutionOrchestrator`, `BatchRunner` structs, `tokio`, `JoinSet`)
- **Reality:** The repo is **Python-based** (has `generate_report.py`, uses DSPy library, GEPA framework — all Python). Ernest's evolution loop is a fictional Rust reimagining, not a description of the source system.

### 2. **No GPU / no fine-tuning — our docs imply fine-tuning**
- **Our docs claim:** Trajectories export as "OpenAI-compatible JSONL for fine-tuning" implying model weight updates
- **Reality:** The repo explicitly states **"No GPU training required. Everything operates via API calls — mutating text, evaluating results."** The system improves *text artifacts* (skills, prompts, tool descriptions), not model weights. There is no fine-tuning pipeline.

### 3. **Evolution mechanism is different**
- **Our docs describe:** A 4-phase pipeline: collect trajectories → LLM pattern analysis → extract skill draft → validate skill
- **Reality:** The repo uses **GEPA (Genetic-Pareto Prompt Evolution)** — a population-based genetic algorithm with fitness scoring across multiple metrics (Pareto-optimal selection). It's not a simple linear 4-step pipeline but an iterative population/mutation/selection loop with multiple generations.

### 4. **What gets evolved is broader in the source**
- **Our docs focus on:** Skill documents only
- **Reality:** The repo evolves (or plans to evolve) **4 artifact types**: skills, tool descriptions, system prompts, and agent code — with skills being only Phase 1 of 5 planned phases.

### 5. **Cost/economics not mentioned**
- **Our docs:** No mention of cost
- **Reality:** The repo estimates **~$2–10 per optimization run**, which is relevant operational information for Ernest.

### 6. **Phase completion status misrepresented**
- **Our docs mark self-evolution as "✅ Complete"**
- **Reality:** In the source system, only Phase 1 (skill evolution) is implemented. Phases 2–5 (tool descriptions, system prompts, code evolution) are **not yet built** as of the repo's current state.

### 7. **SQLite trajectory storage is Ernest-specific, not from Hermes**
- **Our docs describe** a detailed SQLite schema with FTS5 for trajectories
- **Reality:** This is Ernest's own design decision. The Hermes self-evolution repo does not use SQLite for trajectory storage — it works with execution traces via DSPy's evaluation framework.

---

## Missing Coverage

Important facts about the real system that our docs don't cover at all:

- **DSPy framework** — The repo is built on DSPy (Stanford's framework for LLM program optimization). This is the core dependency; our docs never mention it.
- **GEPA (Genetic-Pareto Prompt Evolution)** — A multi-objective genetic algorithm that maintains a population of candidate prompts/skills and evolves them across generations. This specific mechanism is absent from our docs.
- **Population-based evolution** — The system maintains a *population* of candidates, not a single draft. Candidates compete via Pareto-front selection (multi-metric, not single pass-rate threshold).
- **Darwinian Evolver component** — A named component in the repo that drives code-level evolution (Phase 5); not mentioned in our docs.
- **Reflective mutation** — The system uses self-critique / reflection to generate mutations, not just random perturbation.
- **Benchmark task suite** — The repo evaluates candidates against a fixed benchmark task set; our docs describe running against randomly sampled tasks which is a meaningfully different evaluation strategy.
- **`hermes evolve compare` CLI** — Baseline vs. evolved comparison tooling exists in the source.
- **Standalone pipeline** — The repo is explicitly a *standalone* optimization tool, separate from the agent runtime. Our docs conflate it with inline agent operation.
- **Phase 1 only is live** — Critical status information: the repo only has skill evolution working; the rest is planned but not implemented.

---

## Verdict (1–5 Accuracy Score)

**Score: 2 / 5**

Our Ernest docs capture the *spirit* of self-evolution (iterative improvement, skill extraction, validation gating) but get the *implementation* significantly wrong:

- The technology stack (Rust vs. Python/DSPy) is entirely invented
- The evolution algorithm (linear 4-phase vs. genetic population-based GEPA) is incorrect
- The claim that it produces fine-tuning data is directly contradicted by "No GPU required"
- The scope of what gets evolved is understated
- The completion status is overstated

The docs read as a plausible *Ernest design spec* rather than an accurate description of the Hermes self-evolution source. They should be clearly labeled as Ernest's own approach inspired by (but diverging from) the upstream Hermes system.
