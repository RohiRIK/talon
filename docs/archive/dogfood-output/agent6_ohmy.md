# Agent 6 — oh-my-claudecode Audit

**Audited:** https://github.com/Yeachan-Heo/oh-my-claudecode
**Against:** `docs/01_Analysis/04_OhMyClaudeCode_Feature_Audit.md`
**Date:** 2026-05-25

---

## Confirmed Accurate

- **Author:** Yeachan-Heo ✅
- **Language:** TypeScript (confirmed in OSSInsight TypeScript rankings) ✅
- **It extends Claude Code CLI** — it is a Claude Code plugin/tool, not a standalone agent ✅
- **npm package exists** — published as `oh-my-claude-sisyphus` (doc didn't mention the npm package name, but the package exists) ✅
- **Custom slash commands are a feature** — `/team` confirmed; the pattern of slash commands is real ✅
- **CLAUDE.md convention** is part of the Claude Code ecosystem that this project operates within ✅

---

## Inaccuracies

### ❌ Fundamental Mischaracterization of Purpose
Our doc describes it as a *"quality-of-life enhancements, prompt templates, and workflow automation"* toolkit — a modest "developer tooling layer." The actual repo headline is **"Teams-first Multi-agent Orchestration for Claude Code."** This is a multi-agent orchestration framework, not a prompt template collection. The core value prop is parallelized agent teams, not spec/plan/review templates.

### ❌ Fabricated Hook API
The TypeScript hook code in section 2.4 (`onBeforeWrite`, `onAfterShell`) appears to be invented. There is no evidence of this API in the repo. The hooks in the repo, if any, follow Claude Code's native hook model (shell commands / HTTP endpoints), not a TypeScript callback object.

### ❌ Prompt Template List Likely Fabricated
The table of templates (spec.md, plan.md, review.md, debug.md, refactor.md, test.md, capture.md) with exact filenames has no source confirmation. The repo's actual focus is the `/team` orchestration command with `advisor` and `executor` roles — not a template library.

### ❌ Wrong Primary Slash Commands
Our doc lists `/spec`, `/plan`, `/review`, `/commit`, `/pr` as the primary commands. The actual primary command shown in all repo descriptions and search snippets is `/team` — e.g., `omc team 2:codex "review auth flow"` and `/team 3:executor "fix all TypeScript errors"`. The workflow-template commands may not exist at all.

### ❌ Misses the "Advisor Flow"
The repo's core architecture involves an **advisor → executor** routing pattern. Every command (whether `omc team` or `/team`) routes through the same advisor flow. This architectural pattern is completely absent from our doc.

### ❌ Package Name Not Noted
The npm package is `oh-my-claude-sisyphus` (the repo was formerly named this). Our doc never mentions the npm package name, which is important for anyone trying to install it.

---

## Missing Coverage

- **Multi-agent team orchestration** — the actual core feature: spawn N parallel Claude Code subagents with role assignments (e.g., `2:codex`, `3:executor`)
- **Advisor/Executor role model** — all commands route through an advisor that plans, then delegates to executors
- **`omc` CLI binary** — there is a CLI tool called `omc` (oh-my-claudecode) with a `team` subcommand
- **Parallel execution** — agents run in parallel, not sequentially; this is the primary perf benefit
- **Oh-my-codex sibling project** — the repo references `oh-my-codex` as a companion for OpenAI Codex users
- **Plugin/marketplace distribution** — it's distributed via Claude Code's plugin marketplace, not just npm
- **Package name:** `oh-my-claude-sisyphus` on npm despite being rebranded

---

## Verdict

**Accuracy Score: 2 / 5**

The doc correctly identifies the author, language (TypeScript), and the fact that it's a Claude Code extension. However, it fundamentally mischaracterizes the tool's purpose (prompt templates vs. multi-agent orchestration), likely fabricates the hook API and template file list, and completely misses the actual core feature: the `/team` command with advisor→executor multi-agent flow. The doc reads as if it was written based on the repo *name* and general Claude Code ecosystem knowledge rather than actual inspection of the repo contents.

**Recommended Action:** Rewrite section 2 from scratch based on the actual repo. The Ernest "spec-plan-build loop" inspiration is fine as a design decision, but should not be attributed to oh-my-claudecode's actual feature set.
