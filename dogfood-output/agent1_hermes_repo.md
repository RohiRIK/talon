# Agent 1 — Hermes Agent Repo Audit

> **Source:** `/home/rohi/.hermes/hermes-agent` (live checkout, Python 3.11)
> **Docs audited:**
> - `docs/01_Analysis/03_Hermes_Agent_Feature_Audit.md`
> - `docs/01_Analysis/06_Capability_Matrix.md`
> **Audit date:** 2026-05-25

---

## Confirmed Accurate

- **Python 3.11+** — confirmed via `__pycache__/*.cpython-311.pyc` throughout
- **SQLite + FTS5** — `hermes_state.py` is the `SessionDB` session store with BM25/FTS5 search
- **Tool names match:** `web_search`, `web_extract`, `terminal`, `read_file`, `write_file`, `patch`, `search_files`, `session_search`, `skills_list`, `skill_view`, `skill_manage`, `delegate_task`, `execute_code`, `cronjob`, `memory`, `todo`, `send_message`, `clarify`, `text_to_speech`, `vision_analyze`, `image_generate` — all confirmed in `toolsets.py`
- **Browser tools:** `browser_navigate`, `browser_snapshot`, `browser_click`, `browser_type`, `browser_scroll`, `browser_back`, `browser_press`, `browser_get_images`, `browser_vision`, `browser_console`, `browser_cdp`, `browser_dialog` — confirmed in `_HERMES_CORE_TOOLS`
- **Tool approval levels:** `AlwaysAsk / AskForDangerous / AlwaysApprove` — confirmed in `tools/approval.py`
- **Profile isolation:** `~/.hermes/profiles/<name>/` convention — confirmed in `hermes_constants.py` (via AGENTS.md: `get_hermes_home()`)
- **Skill hot-reload** — confirmed (`notify` crate equivalent mentioned in AGENTS.md)
- **Mem0 plugin** — present at `plugins/memory/mem0/`
- **Cron scheduler** — `cron/jobs.py`, `cron/scheduler.py`, SQLite-backed
- **ACP adapter** — `acp_adapter/` directory confirmed
- **Home Assistant tools:** `ha_list_entities`, `ha_get_state`, `ha_list_services`, `ha_call_service` — in core toolset
- **Rich + prompt_toolkit** for CLI — confirmed in AGENTS.md
- **Self-evolution / trajectory** — `agent/trajectory.py` confirmed
- **Skill system:** `skills/` (built-in) + `optional-skills/` directories confirmed
- **Config location:** `~/.hermes/config.yaml` (YAML format), API keys in `~/.hermes/.env`
- **Three-shape session_search** — `session_search` in core tools, detailed in AGENTS.md
- **Max iterations default: 90** — confirmed in AIAgent signature
- **MCP tool integration** — `tools/mcp_tool.py`, `tools/mcp_oauth.py`, `agent/transports/hermes_tools_mcp_server.py`

---

## Inaccuracies

### 1. Directory structure is wrong — core loop is NOT in `agent/`
**Doc says:**
```
├── agent/            ← Core loop, state machine, context
├── providers/        ← LLM adapters (Anthropic, OpenAI, custom)
├── memory/           ← SQLite, FTS5, Mem0, skills
```
**Reality:**
- Core loop lives in **`run_agent.py`** (`AIAgent` class, ~12k LOC), not `agent/`
- LLM adapters are in **`agent/transports/`** (not a top-level `providers/` directory)
- There is **no top-level `memory/` directory** — memory is in `hermes_state.py` (SQLite) and `plugins/memory/`
- `agent/` is sub-internals (context engine, compression, rate limiting, etc.), not the main loop

### 2. Agent loop is synchronous, NOT asyncio-driven
**Doc says:** "Python 3.11+, asyncio, aiohttp" implies the agent loop is async.
**Reality:** `run_conversation()` is **entirely synchronous**. The AGENTS.md explicitly states "entirely synchronous, with interrupt checks, budget tracking, and a one-turn grace call." Asyncio is used in the gateway layer, not the agent core loop.

### 3. LLM provider transports differ from what's documented
**Doc says:** "Multi-provider routing: Anthropic, OpenAI, Ollama, OpenRouter"
**Reality:** Actual transport files are: `anthropic.py`, `chat_completions.py` (OpenAI-compatible), `codex.py`, `bedrock.py` (AWS Bedrock). Ollama and OpenRouter are **not** first-class transports — they route through the `chat_completions` OpenAI-compatible transport. Bedrock is a confirmed transport not mentioned in docs.

### 4. WeChat/QQ platforms exist (docs say ❌ DROP)
**Doc (06_Capability_Matrix)** marks WeChat/QQ as `❌ DROP` ("Legal/stability risk").
**Reality:** `gateway/platforms/weixin.py`, `gateway/platforms/wecom.py`, `gateway/platforms/wecom_callback.py`, `gateway/platforms/qqbot/` are all present and active in the source.

### 5. iMessage/BlueBubbles platform exists (docs say ❌ DROP)
**Doc says:** iMessage `❌ DROP` — "macOS-only, not portable."
**Reality:** `gateway/platforms/bluebubbles.py` is present — BlueBubbles is a cross-platform iMessage bridge that Hermes actually supports.

### 6. "~22,000 lines of Python" is a significant undercount
**Doc says:** "Lines of Python: ~22,000"
**Reality:** `run_agent.py` alone is ~12k LOC and `cli.py` is ~11k LOC (per AGENTS.md), totaling ~23k LOC for just those two files. The full codebase with tools, gateway, plugins, and tests (~17k tests across ~900 files) is substantially larger.

### 7. TUI is React/Ink (TypeScript), NOT Rich-based
**Doc says:** CLI/TUI uses "rich + prompt_toolkit" and maps to `ratatui`.
**Reality:** There is a **separate TUI** at `ui-tui/` built with **Ink (React/TypeScript)** — `entry.tsx`, `app.tsx`, `gatewayClient.ts`. This is launched via `hermes --tui`. Rich + prompt_toolkit power the standard CLI, but the full TUI is a TypeScript/React app.

### 8. Skill slash commands inject as user message, not system prompt
**Doc:** Doesn't specify injection point.
**Reality:** Skill slash commands are injected as **user messages** (not system prompt) specifically to preserve prompt caching — an important implementation detail with cost implications.

---

## Missing Coverage

### Platforms not mentioned in docs
The following gateway platforms are present in source but absent from docs:
- **Mattermost** (enterprise chat)
- **DingTalk** (`dingtalk.py`) — Alibaba enterprise messenger
- **Feishu/Lark** (`feishu.py`) — ByteDance enterprise messenger
- **QQBot** (`qqbot/`) — full sub-module with chunked upload, keyboards, crypto
- **BlueBubbles** (`bluebubbles.py`) — iMessage bridge
- **Yuanbao** (`yuanbao.py`) — with proto, media, sticker sub-modules
- **SMS** (`sms.py`)
- **MS Graph Webhook** (`msgraph_webhook.py`) — Microsoft Teams/Outlook
- **API Server** (`api_server.py`) — HTTP REST interface
- **Webhook** (`webhook.py`) — generic inbound webhook

### Kanban multi-agent coordination system
Tools: `kanban_show`, `kanban_list`, `kanban_complete`, `kanban_block`, `kanban_heartbeat`, `kanban_comment`, `kanban_create`, `kanban_link`, `kanban_unblock`. A full `plugins/kanban/` plugin with board dispatcher and worker. Not mentioned anywhere in docs.

### Computer Use tool
`computer_use` tool is in `_HERMES_CORE_TOOLS` (gated on macOS + cua-driver). Not mentioned in docs.

### AWS Bedrock transport
`agent/transports/bedrock.py` — Hermes supports AWS Bedrock as an LLM backend. Not mentioned.

### Codex / Responses API transport
`agent/transports/codex.py` and `codex_responses_adapter.py` — OpenAI Codex / Responses API is a distinct transport. Not mentioned.

### Webhook safe toolset
`_HERMES_WEBHOOK_SAFE_TOOLS` — a constrained subset of tools for untrusted webhook sources (prompt injection protection). Not covered.

### ACP is IDE integration, not just MCP-like
`acp_adapter/` provides **VS Code / Zed / JetBrains** IDE integration. Docs describe it as generic "Agent Communication Protocol (MCP-like)" which misses the actual use case.

### Budget grace call mechanism
The agent loop has a `_budget_grace_call` flag that allows one final LLM call after the iteration budget is exhausted. Important for clean completion, not documented.

### Observability plugin
`plugins/observability/` — metrics, traces, logs plugin. Not mentioned.

### Achievements plugin
`plugins/hermes-achievements/` — gamified achievement tracking. Not mentioned (minor, but notable).

### `--yolo` flag maps to `AlwaysApprove`
Confirmed in source (AGENTS.md + approval.py), mentioned in capability matrix but the specific CLI flag name `--yolo` is not confirmed there.

---

## Verdict

**Accuracy Score: 3 / 5**

The docs capture the *spirit* of Hermes Agent well — core tools, memory architecture, skill system, self-evolution, and scheduling are all correctly described. However, there are several material inaccuracies:

- The directory structure diagram is wrong (core loop location, no `providers/` or `memory/` dirs)
- The agent loop sync vs async characterization is incorrect
- ~10 gateway platforms present in source are not documented
- The Kanban multi-agent system is entirely absent
- WeChat/QQ and iMessage (via BlueBubbles) are marked DROP but are active
- Line count is significantly understated
- The TUI is TypeScript/React, not Python/Rich

The conceptual analysis is strong; the structural/implementation details need a second pass against the live source.
