# Core Agent Loop Design

> **Last corrected:** dogfood pass 2
>
> **Status:** ✅ Complete
> **Category:** Architecture

---

## 1. Loop State Machine

```
         ┌──────────────┐
         │    IDLE      │◄──────────────────────────────┐
         └──────┬───────┘                               │
                │ message arrives                       │ done / max_iter
         ┌──────▼───────┐                               │
         │  BUILD_CTX   │ load memory, assemble prompt  │
         └──────┬───────┘                               │
                │                                       │
         ┌──────▼───────┐                               │
         │   LLM_CALL   │ stream deltas                 │
         └──────┬───────┘                               │
                │ has tool_calls?                       │
           ┌────┴────┐                                  │
           │         │                                  │
   no      ▼    yes  ▼                                  │
    ┌─────────┐ ┌────────────┐                          │
    │ RESPOND │ │ APPROVE    │ check membrane            │
    └────┬────┘ └─────┬──────┘                          │
         │            │ approved                        │
         │     ┌──────▼──────┐                          │
         │     │ EXECUTE     │ run tools in parallel    │
         │     └──────┬──────┘                          │
         │            │                                 │
         │     ┌──────▼──────┐                          │
         │     │  UPDATE_MEM │ append to SQLite         │
         └─────┴──────┬──────┘                          │
                      └────────────────────────────────►┘
                          loop back to BUILD_CTX
```

---

## 2. Core Agent Struct

```rust
pub struct Agent {
    pub id: Uuid,
    pub config: Arc<AgentConfig>,
    pub llm: Arc<dyn LlmProvider>,
    pub tools: Arc<ToolRegistry>,
    pub memory: Arc<MemoryStore>,
    pub session: Arc<SessionStore>,
    pub approval: ApprovalMembrane,
    pub limits: AgentLimits,
    pub event_tx: broadcast::Sender<AgentEvent>,
}

pub struct AgentConfig {
    pub model: String,
    pub system_prompt: String,
    pub max_iterations: u32,
    pub max_tokens: u32,
    pub approval_level: ApprovalLevel,
    pub tools_enabled: Vec<String>,
    pub profile_dir: PathBuf,
}
```

---

## 3. Context Builder

```rust
pub struct ContextBuilder<'a> {
    agent: &'a Agent,
    session_id: Uuid,
}

impl<'a> ContextBuilder<'a> {
    pub async fn build(&self) -> Result<Vec<Message>, AgentError> {
        let mut messages = vec![];

        // 1. System prompt + MEMORY.md injection
        let system = self.build_system_prompt().await?;
        messages.push(Message::system(system));

        // 2. Last N conversation turns from SQLite
        let history = self.agent.session
            .load_recent(self.session_id, self.agent.config.context_window_turns)
            .await?;
        messages.extend(history);

        // 3. Skill summaries (if skill header loaded)
        // injected via system prompt template

        Ok(messages)
    }

    async fn build_system_prompt(&self) -> Result<String, AgentError> {
        let memory_md = self.agent.memory.load_memory_md().await?;
        let user_md = self.agent.memory.load_user_md().await?;
        let skills = self.agent.memory.list_skill_summaries().await?;
        let agents_md = self.load_agents_md().await.unwrap_or_default();

        // askama template render
        let tpl = SystemPromptTemplate {
            base: &self.agent.config.system_prompt,
            memory: &memory_md,
            user_profile: &user_md,
            skills: &skills,
            project_context: &agents_md,
        };
        Ok(tpl.render()?)
    }
}
```

---

## 4. Tool Dispatcher

```rust
pub async fn dispatch_tool_calls(
    agent: &Agent,
    calls: Vec<ToolCall>,
    session_id: Uuid,
) -> Vec<ToolResult> {
    // Run all approved calls in parallel
    let futs = calls.into_iter().map(|call| {
        let agent = agent.clone();
        async move {
            // Check approval membrane
            let approved = agent.approval
                .check(&call, &agent.config.approval_level)
                .await;

            match approved {
                ApprovalDecision::Approved => {
                    let ctx = ToolContext {
                        session_id,
                        call_id: call.id.clone(),
                        args: call.arguments.clone(),
                        agent_config: agent.config.clone(),
                        memory: agent.memory.clone(),
                        event_tx: agent.event_tx.clone(),
                    };
                    let result = agent.tools.execute(&call.name, ctx).await;
                    ToolResult::from_result(call.id, result)
                }
                ApprovalDecision::Denied(reason) => {
                    ToolResult::error(call.id, reason)
                }
                ApprovalDecision::NeedUserApproval(prompt) => {
                    // send prompt to gateway, await response
                    todo!("approval flow")
                }
            }
        }
    });

    futures::future::join_all(futs).await
}
```

---

## 5. Approval Membrane

```rust
// Canonical approval levels — see talon-core/src/tools/mod.rs
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ApprovalLevel {
    Safe,          // Always execute
    Confirmation,  // Ask once, remember answer
    Required,      // Always ask, no memory
    Blocked,       // Never execute
}

pub enum ToolRisk {
    ReadOnly,       // web_search, read_file
    SafeWrite,      // write_file, create_dir
    Dangerous,      // terminal, browser navigation
    Irreversible,   // git push, deploy, delete
}

pub struct ApprovalMembrane {
    pub level: ApprovalLevel,
    // per-tool overrides loaded from config
    pub overrides: HashMap<String, ApprovalLevel>,
}
```

---

## 6. Event Bus

```rust
// Canonical AgentEvent definition lives in talon-core/src/events.rs
// (see doc 31_Streaming_And_Realtime_Output.md for full enum).
// Reproduced subset used by the agent loop:
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    // LLM is producing text
    TextDelta { content: String },
    TextComplete { content: String },

    // LLM requested a tool call
    ToolCallStart { id: String, name: String },
    ToolCallArgs { id: String, args_chunk: String },
    ToolCallComplete { id: String, name: String, args: Value },

    // Tool executed
    ToolResult { id: String, name: String, output: String, is_error: bool },

    // Approval needed
    ApprovalRequired { id: Uuid, tool: String, description: String, risk: String },
    ApprovalDecision { id: Uuid, approved: bool },

    // Agent loop lifecycle
    IterationStart { n: u32 },
    Done { final_response: String, iterations: u32, usage: UsageSummary },
    Error { message: String, code: String },
}
```

Gateway adapters subscribe to this [broadcast channel](../06_Concurrency/51_Channel_Patterns.md) and forward relevant events to their platform (e.g., Telegram shows tool activity as chat messages).
---

## Related Documents

### Depends On
- [Tool System Architecture](16_Tool_System_Architecture.md)
- [LLM Provider Abstraction](../05_API_Bindings/41_LLM_Provider_Abstraction.md)
- [Plugin & Skill Architecture](17_Plugin_And_Skill_Architecture.md)
- [Error Handling Strategy](../06_Concurrency/54_Error_Handling_Strategy.md)

### Used By
- [Agent Loop Implementation](../04_Core_Features/29_Agent_Loop_Implementation.md)
- [Cron Scheduler](../04_Core_Features/33_Cron_Scheduler.md)
- [Self-Evolution Loop](../04_Core_Features/39_Self_Evolution_Loop.md)
- [Subagent Delegation](../04_Core_Features/37_Subagent_Delegation.md)

### See Also
- [State Machine & Lifecycle](14_State_Machine_And_Lifecycle.md)
- [Async Tool Execution](../06_Concurrency/50_Async_Tool_Execution.md)
- [Streaming & Realtime Output](../04_Core_Features/31a_Streaming_And_Realtime_Output.md)

