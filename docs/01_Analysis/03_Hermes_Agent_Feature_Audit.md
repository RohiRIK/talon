# Hermes Agent Feature Audit

> **Last corrected:** dogfood pass 4
>
> **Status:** ✅ Complete
> **Category:** Analysis

---

## 1. What is Hermes Agent?

Hermes Agent is a Python-based autonomous AI agent framework from NousResearch.
It is the primary Python reference for Talon and shares significant conceptual
overlap with OpenClaw — both are tool-using LLM agents with memory and scheduling.

Repositories:
- `https://github.com/NousResearch/hermes-agent`
- `https://github.com/NousResearch/hermes-agent-[self-evolution](../04_Core_Features/39_Self_Evolution_Loop.md)`

Docs: `https://hermes-agent.nousresearch.com/docs/`

---

## 2. Architecture Summary

```
Hermes Agent
├── run_agent.py      ← AIAgent class — core conversation loop (~12k LOC, synchronous)
├── model_tools.py    ← Tool orchestration, discover_builtin_tools()
├── agent/            ← Provider adapters, memory plugins, caching, compression
├── tools/            ← Tool implementations (50+)
├── gateway/          ← Messaging gateway — 22+ platform adapters
│   └── platforms/    ← telegram, discord, slack, whatsapp, signal, matrix,
│                        homeassistant, mattermost, email, sms, dingtalk, wecom,
│                        weixin, feishu, qqbot, bluebubbles, yuanbao, webhook,
│                        api_server, and more
├── cron/             ← Scheduled jobs
├── plugins/          ← Extensibility layer
│   ├── memory/       ← Memory-provider plugins (honcho, mem0, supermemory, …)
│   ├── model-providers/ ← Inference backend plugins (openrouter, anthropic, gmi, …)
│   ├── kanban/       ← Multi-agent board dispatcher + worker plugin
│   └── observability/ ← Metrics / traces / logs plugin
├── ui-tui/           ← Ink (React/TypeScript) terminal UI — `hermes --tui`
├── tui_gateway/      ← Python JSON-RPC backend for the TUI
└── self_evolution/   ← Skill auto-generation loop
```

Tech stack: Python 3.11+, SQLite, python-telegram-bot, Playwright, [fastembed](../07_Memory_System/59_Embedding_Retrieval.md),
Qdrant (optional). **Agent loop is synchronous** (`run_conversation()` in
`run_agent.py`); the gateway layer is async. TUI is TypeScript/React (Ink) in
`ui-tui/`; the CLI uses Rich + prompt_toolkit.

---

## 3. Feature Inventory

### 3.1 Core Agent Features
| Feature | Details | Talon Verdict |
|---------|---------|----------------|
| Multi-provider routing | Anthropic, OpenAI, Ollama, OpenRouter, AWS Bedrock, Codex (`codex_responses` api_mode) | ✅ Keep |
| Streaming + tool calls | SSE parsing with partial tool call assembly | ✅ Keep |
| Context window management | Sliding window + summarization | ✅ Keep |
| [Profile isolation](../04_Core_Features/40_Profile_Isolation.md) | Per-profile config, memory, cron, skills | ✅ Keep |
| Self-evolution loop | Agent generates/tests new skills autonomously | ✅ Keep (Phase 2) |
| Trajectory generation | Batch synthetic data creation | ✅ Keep (Phase 2) |
| Skill hot-reload | Skills reload without restart | ✅ Keep |
| Tool approval levels | Safe / Confirmation / Required / Blocked | ✅ Keep |

### 3.2 Memory System (Key Differentiator)
| Feature | Details | Talon Verdict |
|---------|---------|----------------|
| [SQLite FTS5](../07_Memory_System/55_SQLite_FTS5_In_Rust.md) sessions | Full message history with BM25 search | ✅ Keep |
| Three-shape session_search | Browse / Scroll / Discovery | ✅ Keep exactly |
| Persistent memory notes | Key/value user notes in SQLite | ✅ Keep |
| Mem0 self-hosted | Qdrant + Ollama vector memory | ✅ Keep as optional |
| Skill file system | Markdown files as procedural memory | ✅ Keep |
| User profile | Persistent user facts file | ✅ Keep |
| [Cross-session context](../07_Memory_System/56a_Cross_Session_Context.md) | Bookend context for recall | ✅ Keep |
| Session lineage deduplication | Parent/child session tracking | ✅ Keep |

### 3.3 Scheduling & Autonomy
| Feature | Details | Talon Verdict |
|---------|---------|----------------|
| Cron jobs | SQLite-backed, persist across restarts | ✅ Keep |
| [Delegate_task](../04_Core_Features/37_Subagent_Delegation.md) | [Parallel subagent spawning](../06_Concurrency/51a_Parallel_Subagent_Spawning.md) | ✅ Keep (Tokio) |
| Background processes | Async subprocess management | ✅ Keep |
| notify_on_complete | Process exit → agent notification | ✅ Keep |
| watch_patterns | stdout pattern → notification | ✅ Keep |
| [ACP protocol](../05_API_Bindings/48_ACP_Protocol_Integration.md) | Agent Communication Protocol (MCP-like) | ✅ Keep |

### 3.4 Delivery Channels
| Channel | Library | Talon Target |
|---------|---------|--------------| 
| Telegram | python-telegram-bot | [teloxide](../05_API_Bindings/45_Telegram_Integration.md) |
| CLI | rich + prompt_toolkit | [ratatui](../04_Core_Features/36_TUI_Implementation.md) |
| TUI | Ink (React/TypeScript) in `ui-tui/` | ratatui |
| HTTP | FastAPI | axum |
| Discord | discord.py | serenity |
| Signal | semaphore-bot | defer |
| Matrix | matrix-nio | defer |
| Slack | gateway platform | reqwest Bolt |
| WhatsApp | gateway platform | HTTP bridge |
| Home Assistant | gateway platform | HTTP API |
| Mattermost | gateway platform | defer |
| Email (IMAP/SMTP) | gateway platform | lettre |
| SMS | gateway platform | defer |
| DingTalk / WeCom / Weixin / Feishu / QQBot | gateway platforms | defer |
| BlueBubbles / Yuanbao / Webhook / API server | gateway platforms | defer |

### 3.6 Multi-Agent & Advanced Tools
| Feature | Details | Talon Verdict |
|---------|---------|----------------|
| Kanban multi-agent | Board dispatcher + worker plugin in `plugins/kanban/` | ✅ Keep (Phase 2) |
| `computer_use` tool | Desktop/GUI automation tool | ✅ Keep |
| Observability plugin | Metrics / traces / logs in `plugins/observability/` | ✅ Keep |

### 3.5 Self-Evolution (Unique Feature)
Hermes Agent can autonomously improve itself:
1. `trajectory_generation` — runs tasks, records tool call sequences
2. `skill_extraction` — analyzes trajectories, writes new skill docs
3. `skill_validation` — tests extracted skills against real tasks
4. `skill_integration` — merges validated skills into [skill store](../07_Memory_System/57_Skill_Store.md)

Talon Phase 2 will replicate this in Rust with a dedicated `talon-evolution` crate.

---

## 4. What to Drop / Replace

| Component | Reason |
|-----------|--------|
| `asyncio` in gateway layer | Tokio handles this better; agent loop itself is already synchronous |
| `python-telegram-bot` polling mode | teloxide has native async + webhook |
| `rich` console rendering | ratatui provides true TUI |
| `playwright` Python bindings | `[chromiumoxide](../04_Core_Features/32_Browser_Tool.md)` (Rust) is faster |
| `fastembed` Python | `fastembed-rs` (Rust) — same models, native |
| Global `asyncio.Lock` patterns | Rust's `tokio::sync::Mutex` is type-safe |
| `pickle`/`shelve` state storage | All state goes to SQLite |
| Dynamic `importlib` for plugins | WASM plugins via `wasmtime` |
| `subprocess.run` for tools | `tokio::process::Command` |
| Manual JSON schema dicts | `schemars` derive macros |
| `logging` + `loguru` mix | `tracing` subscriber unified |

---

## 5. Hermes Agent Python Patterns → Rust

### 5.1 Async Tool Execution

```python
# Hermes Agent Python
async def execute_tool(self, name: str, args: dict) -> ToolResult:
    tool = self.registry.get(name)
    if tool is None:
        return ToolResult(error=f"Unknown tool: {name}")
    try:
        result = await asyncio.wait_for(
            tool.execute(args),
            timeout=tool.timeout
        )
        return ToolResult(content=result)
    except asyncio.TimeoutError:
        return ToolResult(error=f"Tool {name} timed out")
    except Exception as e:
        return ToolResult(error=str(e))
```

```rust
// Talon Rust equivalent
async fn execute_tool(&self, name: &str, args: Value, ctx: &ToolContext)
    -> Result<ToolResult, ToolError>
{
    let tool = self.registry.get(name)
        .ok_or_else(|| ToolError::NotFound(name.to_string()))?;

    let timeout = tool.timeout();
    tokio::time::timeout(timeout, tool.execute(args, ctx))
        .await
        .map_err(|_| ToolError::Timeout { tool: name.to_string(), after: timeout })?
}
```

### 5.2 Profile Isolation

```python
# Python: profile isolation via directory convention
PROFILE_DIR = Path.home() / ".hermes" / "profiles" / profile_name
```

```rust
// Rust: compile-time checked path resolution
pub struct Profile {
    pub name: String,
    pub dir: PathBuf,
}

impl Profile {
    pub fn load(name: &str) -> Result<Self, ProfileError> {
        let dir = dirs::home_dir()
            .ok_or(ProfileError::NoHomeDir)?
            .join(".talon")
            .join("profiles")
            .join(name);

        if !dir.exists() {
            return Err(ProfileError::NotFound(name.to_string()));
        }

        Ok(Self { name: name.to_string(), dir })
    }

    pub fn db_path(&self) -> PathBuf { self.dir.join("talon.db") }
    pub fn skills_dir(&self) -> PathBuf { self.dir.join("skills") }
    pub fn config_path(&self) -> PathBuf { self.dir.join("config.toml") }
}
```

---

## 6. Hermes Agent Quantified

```
Lines of Python: ~23,000+ (run_agent.py alone ~12k LOC)
Tool implementations: 50+
Test coverage: ~40%
PyPI dependencies: 65 (direct)
Startup time: ~3.5s (Python import + asyncio + model loads)
Memory RSS (idle): ~220MB (including Telegram polling)
```

---

## 7. Unique Hermes Contributions to Talon

1. **Three-shape `session_search`** — browse/scroll/discovery with bookend context is the single most important UX feature. Implement exactly.

2. **[Skill system](../04_Core_Features/34a_Skill_System.md) design** — `SKILL.md` frontmatter + body, categories, pinning, curator — elegant, keep it all.

3. **Profile isolation** — separate DB + skills + config per persona; essential for multi-tenant homelab deployments.

4. **Self-evolution** — the trajectory → skill extraction loop is genuinely novel; worth implementing in Phase 2.

5. **Memory routing skill** — the concept of a "memory routing skill" that teaches the agent WHERE to store things is a meta-level pattern Talon should adopt.

6. **Cron deliver targets** — `origin`, `local`, `all`, `platform:chat_id:thread_id` — clean delivery abstraction worth keeping verbatim.
---

## Related Documents

### Depends On
- [Source Ecosystem Overview](01_Source_Ecosystem_Overview.md)

### See Also
- [Python Pain Points](08_Python_Pain_Points.md)
- [Capability Matrix](06_Capability_Matrix.md)
- [Plugin & Skill Architecture](../02_Architecture/17_Plugin_And_Skill_Architecture.md)
- [Self-Evolution Loop](../04_Core_Features/39_Self_Evolution_Loop.md)

