# Ernest AI Docs — Dogfood Master Report

> **Audit date:** 2026-05-25
> **Agents run:** 6 of 8 (agent7, agent8 not yet produced)
> **Sources audited:** Hermes Agent repo (live), Hermes docs site, Hermes self-evolution repo, OpenClaw repo, OpenClaw site, oh-my-claudecode repo

---

## Executive Summary

Six audit agents independently compared the Ernest documentation against live source code, official docs sites, and GitHub repositories. The findings are sobering: **across all six sources, not a single doc scored above 3/5 for accuracy**, and two scored 2/5. While the Ernest docs correctly capture many high-level concepts (skill system, SQLite/FTS5 memory, profile isolation, cron scheduling), they contain a critical cluster of fabricated implementation details, wrong technology stacks, materially incomplete platform/channel coverage, and several internal contradictions between Ernest docs themselves.

The most serious problems are in the self-evolution docs (wrong language, fabricated mechanism) and the oh-my-claudecode audit (fabricated hook API, wrong primary feature set). The Hermes Agent docs are the most accurate subset but still have a wrong directory structure, incorrect async claims, and ~10 undocumented gateway platforms.

**Bottom line: The Ernest docs are NOT safe to build from as-is.** They are a useful design sketchpad, but any developer following them to implement Ernest would make incorrect architectural choices in multiple areas.

---

## Top 10 Critical Inaccuracies

Ranked by severity — likelihood × blast radius if a developer builds from this incorrect information.

### 1. 🔴 Self-evolution tech stack is entirely fabricated (Agent 3)
**Ernest says:** Rust implementation with `EvolutionOrchestrator`, `BatchRunner`, `tokio`, `JoinSet`
**Reality:** The Hermes self-evolution repo is **Python-based**, using DSPy and the GEPA (Genetic-Pareto Prompt Evolution) framework. There is no Rust, no tokio, no JoinSet. A developer implementing Ernest's evolution module from these docs would build the wrong system in the wrong language.

### 2. 🔴 Self-evolution mechanism is wrong — not a 4-phase pipeline, and produces no fine-tuning data (Agent 3)
**Ernest says:** Linear 4-phase pipeline (collect → analyze → extract skill → validate); exports OpenAI-compatible JSONL for fine-tuning.
**Reality:** The system uses **GEPA** — a population-based genetic algorithm with Pareto-front multi-metric selection across generations. It explicitly states **"No GPU training required"** and produces no model weight updates. The fine-tuning claim is directly contradicted by the source.

### 3. 🔴 oh-my-claudecode primary feature is completely mischaracterized (Agent 6)
**Ernest says:** A quality-of-life toolkit with prompt templates (`spec.md`, `plan.md`, `review.md`, etc.) and a TypeScript hook API (`onBeforeWrite`, `onAfterShell`).
**Reality:** The tool's headline is **"Teams-first Multi-agent Orchestration for Claude Code."** Its core feature is the `/team` command with an advisor→executor multi-agent flow. The hook API and template list appear to be fabricated. The spec/plan/review templates may not exist at all.

### 4. 🔴 Ernest directory structure diagram is wrong — core loop location is incorrect (Agent 1)
**Ernest says:** `agent/` = core loop; `providers/` = LLM adapters; `memory/` = SQLite/FTS5/Mem0.
**Reality:** Core loop is in `run_agent.py` (~12k LOC). LLM adapters are in `agent/transports/`. There is no top-level `providers/` or `memory/` directory. Any developer using this diagram to understand or replicate the architecture will navigate to the wrong files.

### 5. 🟠 Agent loop sync/async characterization is wrong (Agent 1)
**Ernest says:** Implies asyncio-driven agent loop ("Python 3.11+, asyncio, aiohttp").
**Reality:** `run_conversation()` is **entirely synchronous**. Asyncio is only in the gateway layer. This affects concurrency model decisions and performance expectations throughout Ernest's design.

### 6. 🟠 WeChat/QQ and iMessage (BlueBubbles) marked DROP but are active in source (Agent 1)
**Ernest says:** WeChat/QQ = `❌ DROP` (legal/stability risk); iMessage = `❌ DROP` (macOS-only).
**Reality:** `gateway/platforms/weixin.py`, `wecom.py`, `qqbot/`, and `bluebubbles.py` are all present and active. BlueBubbles is explicitly cross-platform. These are working integrations, not dropped ones — mischaracterizing them could cause Ernest to needlessly re-implement them.

### 7. 🟠 Gateway channel list is severely incomplete — 6 documented vs 22+ real (Agent 2)
**Ernest says:** Telegram, CLI/TUI, HTTP, Discord, Signal, Matrix (6 channels).
**Reality:** Hermes supports 22+ channels including Slack, WhatsApp, SMS, Email, Microsoft Teams, LINE, SimpleX, Home Assistant, DingTalk, Mattermost, Feishu/Lark, Yuanbao, QQBot, and more. Coverage is ~27% of actual platform surface.

### 8. 🟠 Self-evolution completion status overstated (Agent 3)
**Ernest says:** Self-evolution marked as `✅ Complete`.
**Reality:** Only Phase 1 (skill evolution) is implemented. Phases 2–5 (tool descriptions, system prompts, code evolution) are planned but not built. Ernest's roadmap should not treat this as done.

### 9. 🟡 OpenClaw framed as "agent framework" — primary identity is "gateway" (Agent 4)
**Ernest says:** OpenClaw is "a TypeScript-based autonomous AI agent framework."
**Reality:** OpenClaw's own docs lead with: "a self-hosted gateway that connects your favorite chat apps to AI coding agents." The gateway/routing layer is the primary product. This framing inversion affects how Ernest should think about what it is inheriting from OpenClaw.

### 10. 🟡 OpenClaw uses NestJS — not just raw Node.js/TypeScript (Agent 5)
**Ernest says:** Tech stack is "Node.js 20, TypeScript 5, Anthropic SDK, LangChain (partial), SQLite, Telegraf."
**Reality:** The backend framework is **NestJS** — an opinionated framework with decorators, dependency injection, and modules. This is a major architectural fact missing from Ernest's stack description and would affect migration effort estimates significantly.

---

## Top 10 Missing Coverage Gaps

Features and systems with zero or severely inadequate coverage in Ernest docs.

### 1. Kanban multi-agent coordination system (Agent 1)
`plugins/kanban/` is a full multi-agent coordination plugin with 9 tools (`kanban_show`, `kanban_list`, `kanban_complete`, `kanban_block`, `kanban_heartbeat`, `kanban_comment`, `kanban_create`, `kanban_link`, `kanban_unblock`). Not mentioned anywhere in any Ernest doc reviewed.

### 2. GEPA / DSPy self-evolution engine (Agent 3)
The actual evolution mechanism — a population-based genetic algorithm using Stanford's DSPy framework — is entirely absent. Ernest docs describe a fictional linear pipeline instead.

### 3. Web dashboard for local agent management (Agent 2)
Hermes has a browser-based web dashboard for managing cron jobs, run history, and config. Not mentioned anywhere in Ernest docs.

### 4. Profile distributions (git-packaged shareable agents) (Agent 2)
A feature that packages a complete Hermes agent (skills, cron, MCP, config, personality) as a cloneable git repo. Ernest docs don't cover this distribution/sharing model.

### 5. Docker as terminal execution backend (Agent 2)
A documented mode where all shell commands run inside a persistent Docker sandbox that survives across `/new` and subagents — important for security/isolation. Not in Ernest docs.

### 6. OpenClaw's ClawHub skills marketplace (Agents 4, 5)
OpenClaw has a **community skills marketplace** called ClawHub. Ernest docs describe the skill system as files but never reference the public registry/marketplace model.

### 7. oh-my-claudecode advisor→executor multi-agent architecture (Agent 6)
The actual core of oh-my-claudecode — the advisor flow, parallel executor agents, and `/team N:role` command syntax — is completely absent from Ernest docs, replaced by fabricated content.

### 8. AWS Bedrock and native Gemini/Azure Foundry transports (Agents 1, 2)
Three LLM backends missing from Ernest's provider list: AWS Bedrock (`bedrock.py`), native Google Gemini adapter (multi-turn, multimodal), and Microsoft Azure Foundry. Ernest only lists Anthropic, OpenAI, Ollama, OpenRouter.

### 9. `computer_use` tool and webhook safe toolset (Agent 1)
`computer_use` is in `_HERMES_CORE_TOOLS` (gated on macOS + cua-driver) and `_HERMES_WEBHOOK_SAFE_TOOLS` is a prompt-injection-protection subset. Neither is documented — the latter has security implications.

### 10. `@` context references, `/indicator`, `[SILENT]` cron flag, mid-turn slash commands (Agent 2)
Several user-facing UX features completely undocumented: `@file`/`@url` context injection, `/indicator` for Enter behavior, `[SILENT]` flag for cron jobs, and mid-turn queue/steer/interrupt controls.

---

## Per-Source Accuracy Scores

**Score scale:** 1 = mostly fabricated / 2 = significant errors / 3 = correct core, wrong details / 4 = mostly correct / 5 = fully verified

| Agent | Source Audited | Ernest Doc(s) Checked | Score | Key Issue |
|-------|---------------|----------------------|-------|-----------|
| **Agent 1** | Hermes Agent repo (live source) | `03_Hermes_Agent_Feature_Audit.md`, `06_Capability_Matrix.md` | **3/5** | Wrong dir structure, sync/async error, ~10 undocumented platforms, TUI is TypeScript not Python |
| **Agent 2** | Hermes docs site (hermes-agent.nousresearch.com) | `03_Hermes_Agent_Feature_Audit.md`, `34_Skill_System.md`, `55_SQLite_FTS5_In_Rust.md` | **3/5** | 6 vs 22+ gateway channels, unverified metrics presented as facts, missing web dashboard/profile distributions |
| **Agent 3** | Hermes self-evolution repo (GitHub) | `39_Self_Evolution_Loop.md`, `38_Batch_Trajectory_Generation.md` | **2/5** | Wrong language (Rust vs Python), fabricated fine-tuning claim, wrong algorithm (linear vs GEPA), status overstated |
| **Agent 4** | OpenClaw repo (GitHub) | `02_OpenClaw_Feature_Audit.md` | **3/5** | Two broken doc references, gateway-first identity inverted, missing SOUL.md / Onboard CLI, unverified metrics |
| **Agent 5** | OpenClaw site (openclaw.ai) | `02_OpenClaw_Feature_Audit.md`, `09_Rust_Migration_Tradeoffs.md` | **3/5** | Missing NestJS, WhatsApp demoted to "deferred" when it's primary, internal contradictions on startup time (2.1s vs 0.8s) and RSS (180MB vs 80MB) |
| **Agent 6** | oh-my-claudecode repo (GitHub) | `04_OhMyClaudeCode_Feature_Audit.md` | **2/5** | Fundamental mischaracterization of purpose, fabricated hook API, fabricated template list, wrong primary commands |
| **Agent 7** | *(not yet run)* | — | — | — |
| **Agent 8** | *(not yet run)* | — | — | — |

**Aggregate score: 2.67 / 5** *(average of 6 completed agents)*

---

## Overall Verdict

### ❌ NOT safe to build from as-is — requires targeted major corrections

**The Ernest docs are NOT a reliable implementation guide in their current state.** They function well as a *design vision document* and are useful for understanding the problem space, but contain too many material inaccuracies to be used as a build spec without correction.

**What the docs get right:**
- High-level architecture concept (gateway + agent core + skill system + memory)
- Core tool names and approval membrane model
- Profile isolation pattern
- SQLite/FTS5 memory architecture (conceptually)
- Skill system lifecycle (create, pin, validate, evolve)
- Cron scheduling model

**What the docs get wrong at a build-breaking level:**
- Self-evolution implementation (wrong language, wrong algorithm, fabricated fine-tuning)
- oh-my-claudecode feature inventory (largely fabricated)
- Directory structure of the core system (wrong paths)
- Sync vs async characterization of agent loop
- Gateway platform coverage (~73% of platforms missing)
- LLM provider coverage (3 missing backends)

**Risk:** A developer building Ernest from these docs as-is would:
1. Implement a wrong self-evolution system in Rust with a non-existent 4-phase pipeline
2. Expect fine-tuning capabilities that don't exist in the reference system
3. Build a directory structure that doesn't match the reference
4. Miss 16+ gateway platforms already implemented upstream
5. Plan a migration from oh-my-claudecode based on features that don't exist

---

## Priority Fix List

Ordered by: (severity of error × how foundational the doc is to other docs)

### 🔴 P0 — Fix immediately before any implementation work

1. **`39_Self_Evolution_Loop.md` + `38_Batch_Trajectory_Generation.md`** — Rewrite from scratch. Remove all Rust/tokio references. Document the actual Python/DSPy/GEPA architecture. Remove the fine-tuning claim. Mark Phases 2–5 as not yet implemented. *Foundational to Ernest's core differentiator.*

2. **`04_OhMyClaudeCode_Feature_Audit.md`** — Rewrite Section 2 entirely. Replace fabricated hook API and template list with actual content: `/team` command, advisor→executor pattern, parallel multi-agent execution, `omc` CLI. *Any inspiration Ernest draws from this tool is currently based on fiction.*

3. **`03_Hermes_Agent_Feature_Audit.md` — Directory structure diagram** — Correct `agent/` → `run_agent.py`, remove `providers/`, remove `memory/` top-level, add `agent/transports/`. *Affects every developer reading the codebase map.*

### 🟠 P1 — Fix before gateway/platform design is finalized

4. **`06_Capability_Matrix.md` — Gateway platform table** — Add 16+ missing platforms: Mattermost, DingTalk, Feishu/Lark, QQBot, BlueBubbles, Yuanbao, SMS, MS Graph Webhook, API Server, Webhook, LINE, SimpleX, WhatsApp (promote from deferred), Email. Correct WeChat/QQ and iMessage from `❌ DROP` to `✅ Active`.

5. **`03_Hermes_Agent_Feature_Audit.md` — Async claim** — Correct "asyncio-driven agent loop" to "synchronous agent loop; asyncio only in gateway layer." *Affects concurrency model throughout Ernest design.*

6. **`02_OpenClaw_Feature_Audit.md` — Primary identity + NestJS** — Reframe as gateway-first, not agent-framework-first. Add NestJS to tech stack. Resolve internal contradiction on startup time (2.1s vs 0.8s) and RSS (180MB vs 80MB). *Affects migration effort estimates.*

### 🟡 P2 — Fill coverage gaps before implementation of those systems

7. **Add new doc: `40_Kanban_Multiagent_Coordination.md`** — Document the full Kanban plugin: 9 tools, board dispatcher, worker pattern. *Entire multi-agent coordination surface is undocumented.*

8. **Add new doc: `41_Profile_Distributions.md`** — Document git-packaged shareable agent profiles, web dashboard, Docker execution backend. *Key deployment/sharing features completely missing.*

9. **Add to provider docs: Bedrock, Gemini, Azure Foundry** — Ernest's LLM provider list is missing 3 backends. Update `03_Hermes_Agent_Feature_Audit.md` §LLM Providers and any routing/transport docs. *Affects provider selection decisions.*

10. **Fix all broken doc references** — `09_Keep_Edit_Drop_Analysis.md` does not exist; `21_Migration_Phases_Overview.md` should be `21_Migration_Roadmap.md`. Audit all cross-references in `01_Analysis/` for broken links before the doc set grows further. *Causes agent confusion and broken navigation.*

---

*Report generated by Master Audit Agent — 2026-05-25*
*Based on findings from Agents 1–6. Agents 7–8 pending.*
