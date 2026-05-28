# oh-my-claudecode Feature Audit

> **Status:** ✅ Complete
> **Category:** Analysis

---

## 1. What is oh-my-claudecode?

`oh-my-claudecode` (OMC) is a **teams-first multi-agent orchestration framework** by Yeachan-Heo that runs on top of Claude Code. It is not a simple tooling layer — it is a full agent coordination system that spawns, manages, and routes work across parallel Claude Code subagent instances.

- **Repository:** `https://github.com/Yeachan-Heo/oh-my-claudecode`
- **npm package:** `oh-my-claude-sisyphus` (the npm name is a legacy artifact; all branding uses "oh-my-claudecode")
- **CLI binary:** `omc`
- **Install:** `npm i -g oh-my-claude-sisyphus@latest`
- **Sibling project:** `oh-my-codex` (OMX) — the same pattern adapted for OpenAI Codex CLI, published as `oh-my-codex` on npm
- **Scale:** ~34,600 GitHub stars as of mid-2026

The core philosophy is "zero configuration, team-first": describe a task in natural language and OMC routes it through specialized agents, potentially in parallel, until completion.

---

## 2. Core Features

### 2.1 The `/team` Command — Primary Orchestration Surface

The canonical multi-agent entry point is `/team`. The syntax spawns N parallel Claude Code subagent workers assigned a named role:

```
/team <N>:<role> "<task description>"
```

**Examples:**

```bash
# Inside a Claude Code / OMC session
/team 3:executor "fix all TypeScript errors in src/"
/team 2:codex "review the auth flow for security issues"

# Equivalent CLI form
omc team 2:codex "review auth flow"
omc team 3:executor "refactor payment module"
```

- `N` — number of parallel worker agents to spawn
- `role` — agent persona/specialization (e.g., `executor`, `codex`, `reviewer`)
- Each worker runs as an independent Claude Code subprocess coordinating through a shared task queue

Additional team management commands:
```bash
omc team status <team-name>         # Check team progress
omc team shutdown <team-name>       # Graceful shutdown
omc team shutdown <team-name> --force  # Force kill
omc team api claim-task --input '{"team_name":"...","task_id":"1","worker":"worker-1"}' --json
```

> **Note:** The `/swarm` alias was removed; all existing prompts using it must migrate to `/team` syntax.

### 2.2 The Advisor → Executor Routing Architecture

Every task in OMC is routed through a **two-phase pipeline**:

```
User Input
    │
    ▼
[Advisor Agent]
  - Analyzes intent
  - Decomposes task into subtasks
  - Selects agent roles and parallelism
    │
    ▼
[Executor Agent(s)]
  - Carry out implementation
  - One or many, running in parallel
  - Write files, run shell commands, call tools
    │
    ▼
[Result Aggregation]
  - Lead session merges outputs
  - Artifacts written to .omc/artifacts/
```

Both `/ask` and `/team` route through the same advisor flow. The advisor prompt can be overridden:

```bash
omc ask claude --agent-prompt executor --prompt "create implementation plan"
```

The environment variables `OMX_ASK_ADVISOR_SCRIPT` and `OMX_ASK_ORIGINAL_TASK` exist as deprecated aliases (sunset planned 2026-06-30) from the earlier OMX-compatible API.

### 2.3 The `/ask` Command — Single-Agent Advisory Queries

Before spawning a full team, operators often query individual specialist agents:

```bash
# Inside session
/ask claude "review this migration plan"
/ask codex "identify architecture risks"

# CLI equivalent
omc ask claude "review this migration plan"
omc ask codex --prompt "identify architecture risks"
omc ask gemini --prompt "propose UI polish ideas"
```

`/ask` runs a single advisory pass — ideal for analysis, risk identification, or generating a plan before committing to `/team` execution.

### 2.4 Deep Interview and Autoresearch

OMC provides research and requirements-gathering workflows:

```bash
/deep-interview --autoresearch "improve startup performance"
/oh-my-claudecode:autoresearch
```

`omc autoresearch` is a **hard-deprecated shim** — the authoritative path is `/deep-interview --autoresearch`. This workflow interviews the user about requirements and performs background research before any implementation begins.

### 2.5 Planning Workflow

Explicit planning before execution:

```bash
/oh-my-claudecode:omc-plan
# or
ralplan   # keyword trigger (replaces deprecated "plan this" natural language trigger)
```

The `plan` keyword trigger was removed; planning must be invoked explicitly.

### 2.6 Native Team Worktree Mode

For file-system isolation, OMC supports a **Native Team Worktree Mode** where each worker operates in its own git worktree. This provides:
- Workspace contract with canonical state-root rules
- Dirty-worktree preservation policy (uncommitted changes are not clobbered)
- Verification checklist before merge

### 2.7 Artifact Storage

All agent outputs are persisted under `.omc/artifacts/`:
```
.omc/artifacts/ask/     ← advisory outputs (markdown)
.omc/artifacts/team/    ← team execution results
```

This gives sessions a persistent memory of what was analyzed and built.

### 2.8 Agent Roster

OMC ships with **32 specialized agents** and **40+ skills**, covering roles like:
- `executor` — implements code changes
- `codex` — code analysis / OpenAI Codex-style review
- `reviewer` — security and quality review
- `gemini` — UI/UX suggestions (routes to Gemini)

---

## 3. Architecture

```
┌─────────────────────────────────────────────────┐
│              OMC Lead Session                    │
│         (Claude Code + OMC plugin)               │
│                                                  │
│   /ask → [Advisor] → analysis artifact           │
│   /team N:role "task"                            │
│       │                                          │
│       ├─► Worker 1 (Claude Code subprocess)      │
│       ├─► Worker 2 (Claude Code subprocess)      │
│       └─► Worker N (Claude Code subprocess)      │
│              │                                   │
│              └─► Shared task queue               │
│                  .omc/artifacts/                 │
└─────────────────────────────────────────────────┘
```

Key architectural properties:
- **Lead + Worker split:** The lead session is the orchestrator; workers are ephemeral subprocesses
- **Parallelism is explicit:** `N` in `/team N:role` directly controls concurrency
- **Role-based specialization:** Each worker is initialized with a role-specific system prompt
- **Provider-agnostic workers:** Workers can target Claude, Codex, or Gemini
- **Persistent artifacts:** Results survive beyond session lifetime

---

## 4. What Talon Borrows from OMC

### 4.1 Advisor → Executor Two-Phase Dispatch (HIGH PRIORITY)

Talon should implement a matching two-phase routing:
1. **Advisor pass:** before executing any multi-step task, route through a planner/advisor persona that decomposes the work
2. **Executor dispatch:** structured subtask objects are dispatched to specialized handlers

This is the most architecturally significant pattern OMC demonstrates. Talon's tool-calling pipeline should support an optional `advisor_mode: true` flag on complex requests.

### 4.2 Explicit Parallelism Declaration

The `/team N:role` syntax is elegant because it makes parallelism **explicit and controllable** by the user. Talon should expose a similar surface — either a TUI command or API parameter — where users can say "run this across N agents with role X."

Example Talon equivalent:
```
:team 3:reviewer "audit all auth endpoints"
```

### 4.3 Role-Based Agent Personas

OMC's 32-agent roster demonstrates the value of specialized roles. Talon should maintain a **role registry** — named agent personas with distinct system prompts — that can be selected at dispatch time rather than using a monolithic agent for everything.

### 4.4 Artifact Persistence Under `.talon/artifacts/`

OMC's `.omc/artifacts/` convention solves the session-memory problem without requiring a database. Talon should adopt a parallel path (e.g., `.talon/artifacts/`) for persisting advisory outputs, plans, and execution summaries across sessions.

### 4.5 AGENTS.md / CLAUDE.md Auto-Injection

OMC inherits the convention of injecting project-root context files. Talon should auto-detect and inject:

```rust
pub async fn load_project_context(workdir: &Path) -> Option<String> {
    for filename in &["AGENTS.md", "CLAUDE.md", ".cursorrules", "CURSOR.md"] {
        let path = workdir.join(filename);
        if path.exists() {
            return tokio::fs::read_to_string(path).await.ok();
        }
    }
    None
}
```

### 4.6 Deep Interview Before Implementation

The `/deep-interview --autoresearch` pattern enforces requirements gathering before coding begins. Talon should surface a `deep-interview` skill that asks clarifying questions before dispatching any substantial implementation task.

---

## 5. Feature Verdict Table

| OMC Feature | Talon Action | Rationale |
|---|---|---|
| `/team N:role "task"` | **Adopt** as `:team N:role` TUI command | Core parallelism primitive |
| Advisor → Executor routing | **Adopt** in dispatch pipeline | Two-phase planning prevents wasted work |
| Role-based agent roster | **Adopt** as role registry | Specialization improves output quality |
| `.omc/artifacts/` persistence | **Adopt** as `.talon/artifacts/` | Session memory without a DB |
| `/ask <provider> "..."` | **Adopt** as `:ask` command | Single-agent advisory before full team |
| AGENTS.md auto-injection | **Adopt** | Established convention, broadly supported |
| `/deep-interview --autoresearch` | **Adopt** as `deep-interview` skill | Forces requirements clarity upfront |
| Native team worktree mode | **Evaluate later** | Useful for parallel file edits; complex to implement |
| omc CLI binary | **Drop** | Talon is standalone Rust, not a Claude Code wrapper |
| oh-my-claude-sisyphus npm package | **Drop** | Talon doesn't run on Node.js |
| Claude Code CLI dependency | **Drop** | Talon is an independent agent, not a plugin |
| 32-agent Gemini/Codex routing | **Partial** | Talon targets Claude; multi-provider later |

---

## 6. Summary

oh-my-claudecode's primary architectural contribution is the **advisor → executor multi-agent pipeline** expressed through the `/team N:role "task"` primitive. This is a fundamentally different model from the original (fabricated) audit's "prompt templates + hooks" description.

The real OMC is a production orchestration system with tens of thousands of users, demonstrating that:
1. **Explicit parallelism** (`N` workers) is a user-controllable parameter, not an internal detail
2. **Role specialization** at dispatch time outperforms monolithic agents
3. **Two-phase advisor/executor** routing catches ambiguity before expensive execution
4. **Artifact persistence** solves cross-session continuity without a full memory system

Talon should treat OMC's architecture as a reference implementation for multi-agent coordination and adopt its core patterns while discarding the Node.js / Claude Code plugin surface layer.
---

## Related Documents

### Depends On
- [Source Ecosystem Overview](01_Source_Ecosystem_Overview.md)

### See Also
- [Capability Matrix](06_Capability_Matrix.md)
- [TypeScript Pain Points](07_TypeScript_Pain_Points.md)

