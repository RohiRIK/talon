# Agent 2 — Hermes Agent Docs Site Audit

*Audited: 2026-05-25 | Sources: hermes-agent.nousresearch.com/docs + GitHub NousResearch/hermes-agent*

---

## Confirmed Accurate

The following facts in our Ernest docs were verified against the live Hermes Agent docs site and GitHub:

### From `03_Hermes_Agent_Feature_Audit.md`
- **Python-based implementation** — Confirmed. Hermes Agent is Python. GitHub repo, installer docs, and python-library guide all confirm Python 3.x.
- **SQLite FTS5 for session memory** — Confirmed. Docs reference FTS5 cross-session recall explicitly.
- **Skill system** — Confirmed. `skills_list()`, `skill_view(name)`, `skill_manage` all exist and behave as documented.
- **Cron jobs (SQLite-backed)** — Confirmed. Cron internals page confirms SQLite-backed scheduler persisting across restarts.
- **Cron deliver targets: `origin`, `local`, `all`, `platform:chat_id:thread_id`** — Confirmed. Cron internals page and automation templates reference these exact target names.
- **Profile isolation** — Confirmed. Each profile gets its own `config.yaml`, `.env`, `SOUL.md`, memories, sessions, skills, cron, and state DB.
- **Self-evolution loop** — Confirmed. Separate `hermes-agent-self-evolution` repo; trajectory generation → skill extraction → validation → integration pipeline.
- **Multi-provider support** — Confirmed. Any OpenAI-compatible `/v1/chat/completions` endpoint works; native Gemini adapter also documented.
- **Gateway channels** — Telegram, Discord confirmed; more platforms than Ernest docs listed (see Inaccuracies below).
- **Profile home dir path: `~/.hermes/profiles/<name>/`** — Confirmed from profiles docs page.
- **`hermes config set` routing API keys to `.env`, other config to `config.yaml`** — Confirmed.
- **ACP (Agent Communication Protocol) for IDE integration** — Confirmed. Integrations page references ACP with VS Code, Zed, JetBrains.

### From `34_Skill_System.md`
- **Skills are Markdown files** — Confirmed. Skill docs confirm `.md` format with YAML frontmatter.
- **`skills_list()` loaded at session start (~3k tokens)** — Confirmed verbatim from Working with Skills guide.
- **`skill_view(name)` loads full content on demand** — Confirmed.
- **`skill_manage` with CRUD actions** — Confirmed. `create`, `patch`, `edit`, `delete`, `write_file`, `remove_file` documented.
- **Pinned skills protected from LLM deletion** — Confirmed. Skills page references pinning.
- **Mandatory skill loading pattern in system prompt** — Confirmed in spirit; docs describe the agent being prompted to load relevant skills.
- **Skills directory path: `~/.hermes/profiles/<name>/skills/`** — Confirmed from profiles page.
- **New skills not visible until new session (current session loader is cached)** — Confirmed explicitly: "the CURRENT session's skill loader is cached — skill_view / skills_list will not see the new skill until a new session."

### From `55_SQLite_FTS5_In_Rust.md` (Ernest Rust design doc — validated that the *Hermes Python* side uses FTS5)
- **FTS5 with BM25 cross-session recall** — Confirmed from Hermes docs.
- **session_search with browse/scroll/discovery shapes** — Confirmed from GitHub commit message: "feat(session_search): single-shape tool with discovery, scroll, brows…"

---

## Inaccuracies

The following items in our Ernest docs contain factual errors or outdated information:

### 1. Gateway channel list is significantly incomplete
**Ernest docs state (Feature Audit §3.4):**
> Telegram, CLI/TUI, HTTP, Discord, Signal, Matrix

**Reality (from Hermes docs messaging page):**
> Telegram, Discord, Slack, WhatsApp, Signal, SMS, Email, Home Assistant, Mattermost, Matrix, DingTalk, Yuanbao, Microsoft Teams, LINE, SimpleX, **and more — 22+ supported platforms total**

Ernest docs listed only 6 channels; the real number is 22+. Several major ones (Slack, WhatsApp, SMS, Email, Microsoft Teams, LINE, SimpleX, Home Assistant) were completely omitted.

### 2. Hermes is NOT described as Python-based with "22,000 lines" anywhere verifiable
**Ernest docs state (Feature Audit §6):**
> Lines of Python: ~22,000, Test coverage: ~40%, PyPI dependencies: 65 (direct), Startup time: ~3.5s, Memory RSS (idle): ~220MB

These appear to be estimates/fabrications — none of these metrics appear on the official Hermes docs site or README. While the Python nature is confirmed, the specific numbers should be flagged as unverified internal estimates.

### 3. Skills system path in Ernest's Skill System doc uses `~/.ernest/` not `~/.hermes/`
**Ernest docs state (34_Skill_System.md §2):**
> `~/.ernest/profiles/<name>/skills/`

This is correct *for Ernest* (the Rust re-implementation), but when cross-referencing against Hermes Agent, the source system uses `~/.hermes/profiles/<name>/skills/`. The doc conflates the two systems — fine if the intent is Ernest's own design, but could cause confusion when used as an audit reference.

### 4. Skill auto-creation trigger "5+ tool calls" is unverified
**Ernest docs state (34_Skill_System.md §4):**
> Task required 5+ tool calls

The Hermes docs describe autonomous skill creation qualitatively ("novel problem", "non-obvious workflow") but the specific "5+ tool calls" threshold is not documented in the official Hermes docs. This may be an inferred or estimated heuristic.

### 5. Memory system file numbered incorrectly
**Ernest docs:** `55_Memory_Architecture_Overview.md` — **FILE DOES NOT EXIST**. The actual files are `55_SQLite_FTS5_In_Rust.md`, `56_Session_Management.md`, etc. The audit was requested for a non-existent file, indicating doc naming drift.

---

## Missing Coverage

The following features and details from the official Hermes docs are not covered (or significantly underrepresented) in our Ernest docs:

### 1. Web Dashboard
Hermes has a **browser-based web dashboard** for managing the agent locally — viewing cron jobs, run history, config, etc. Not mentioned in any Ernest docs reviewed.

### 2. Docker as terminal backend
Beyond running Hermes *in* Docker, there's a documented mode where **Docker acts as the terminal execution backend** — all shell commands run inside a persistent Docker sandbox container that survives across `/new` and subagents. Ernest docs don't discuss this execution isolation model.

### 3. Profile Distributions (git-packaged agents)
A **"profile distribution"** feature packages a complete Hermes agent (skills, cron, MCP connections, config, personality) as a **git repository** that can be shared and cloned. Ernest docs don't cover this sharing/distribution mechanism at all.

### 4. Honcho dialectic user modeling
The docs homepage explicitly calls out **"Honcho dialectic user modeling"** as a key memory feature. Ernest's memory docs (`58_User_Modeling.md` exists but wasn't reviewed in detail) may cover this, but it's not mentioned in the Feature Audit doc.

### 5. MCP (Model Context Protocol) support
The docs reference **MCP connections** as part of profiles. This is separate from ACP. Ernest docs reviewed don't explicitly address MCP vs ACP distinction.

### 6. Python library mode
Hermes can be used as a **Python library** (import `AIAgent` directly) for programmatic use in scripts and pipelines — not just as a CLI. Not covered in Ernest feature inventory.

### 7. `/indicator` and mid-turn steering slash commands
The slash commands reference page documents **mid-turn controls** (queue message, steer, interrupt) and `/indicator` for controlling Enter behavior. Ernest docs don't appear to cover the full slash command surface.

### 8. `[SILENT]` cron flag
Cron troubleshooting docs reference a **`[SILENT]`** flag for cron jobs. Ernest's cron coverage doesn't mention this.

### 9. Context references with `@` syntax
The docs describe pulling in file/URL content using **`@` references** in chat. Not covered in Ernest feature inventory.

### 10. Google Gemini native adapter
Hermes has a **dedicated Gemini adapter** (not just OpenAI-compat passthrough) handling multi-turn tool use, tool-call results, streaming, multimodal inputs, and Gemini response metadata mapping. Ernest's provider section lists "Anthropic, OpenAI, Ollama, OpenRouter" and misses the native Gemini integration.

### 11. Microsoft Foundry / Azure OpenAI provider
An **`azure-foundry` provider** supporting Microsoft Foundry (formerly Azure AI Foundry) and Azure OpenAI. Not in Ernest's provider list.

---

## Verdict

**Accuracy Score: 3 / 5**

**Rationale:**
- Core architecture (Python, SQLite FTS5, skill system, profile isolation, cron, self-evolution) is accurately captured — this is solid foundational work.
- The gateway channel list is badly outdated/incomplete (6 vs 22+ platforms) — a meaningful gap.
- Several unverifiable metric claims (lines of code, test %, startup time) are presented as facts.
- Significant missing features: web dashboard, profile distributions, Docker execution backend, Honcho user modeling, MCP, `@` context references, native Gemini/Azure providers.
- The memory architecture overview doc referenced in the audit request (`55_Memory_Architecture_Overview.md`) doesn't exist — doc naming has drifted.

The Ernest docs are a good-faith effort that captures the most important Hermes concepts correctly, but have notable gaps in breadth (gateway channels, providers) and contain some unverified specifics that should be flagged if used as source-of-truth references.
