use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::mpsc;
use tracing::instrument;

use talon_llm::{ContentBlock, LlmProvider, Message};
use talon_memory::Database;

use crate::{
    approval::{ApprovalLevel, ApprovalMembrane},
    error::CoreError,
    events::AgentEvent,
    state::AgentState,
    tools::dispatcher::{ToolCall, ToolDispatcher},
};

const LLM_TIMEOUT_SECS: u64 = 60;

pub struct Agent {
    provider: Arc<dyn LlmProvider>,
    dispatcher: ToolDispatcher,
    events: mpsc::Sender<AgentEvent>,
    db: Option<Arc<Database>>,
}

impl Agent {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        dispatcher: ToolDispatcher,
        events: mpsc::Sender<AgentEvent>,
    ) -> Self {
        Self {
            provider,
            dispatcher,
            events,
            db: None,
        }
    }

    pub fn with_db(mut self, db: Arc<Database>) -> Self {
        self.db = Some(db);
        self
    }

    /// Run one agent turn: user_message → LLM → tool loop → response.
    ///
    /// State transitions: Idle → Thinking → (CallingTool → Thinking)* → Completed | Failed
    #[instrument(skip(self), fields(session_id = %session_id))]
    pub async fn run(
        &mut self,
        session_id: &str,
        user_message: String,
    ) -> Result<(), CoreError> {
        let membrane = ApprovalMembrane::new(self.events.clone());
        let tool_schemas = self.dispatcher.schemas();
        let mut state = AgentState::Idle;

        self.emit(AgentEvent::Started).await;
        self.persist(session_id, "user", &user_message).await;

        let mut messages = vec![Message::user(user_message)];

        // Initial: Idle → Thinking before the first LLM call.
        state = state.transition(AgentState::Thinking)?;

        loop {
            self.emit(AgentEvent::LlmRequest).await;

            let response = tokio::time::timeout(
                Duration::from_secs(LLM_TIMEOUT_SECS),
                self.provider.complete(&messages, &tool_schemas),
            )
            .await
            .map_err(|_| CoreError::Timeout { secs: LLM_TIMEOUT_SECS })?
            .map_err(|e| CoreError::Llm(e.to_string()))?;

            self.emit(AgentEvent::LlmResponse).await;

            let assistant_json = serde_json::to_value(&response.content)
                .map_err(|e| CoreError::InvalidState(e.to_string()))?;
            messages.push(Message::assistant(assistant_json.clone()));
            self.persist(session_id, "assistant", &assistant_json.to_string()).await;

            // Collect tool calls from this response.
            let tool_calls: Vec<(String, String, serde_json::Value)> = response
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolUse { id, name, input } => {
                        Some((id.clone(), name.clone(), input.clone()))
                    }
                    _ => None,
                })
                .collect();

            if response.stop_reason == "end_turn" || tool_calls.is_empty() {
                self.emit(AgentEvent::Completed).await;
                return Ok(());
            }

            // Process each tool call: Thinking → CallingTool → Thinking (per tool).
            let mut results: Vec<serde_json::Value> = Vec::new();
            for (id, name, args) in tool_calls {
                state = state.transition(AgentState::CallingTool {
                    tool_name: name.clone(),
                })?;

                self.emit(AgentEvent::ToolCalled {
                    id: id.clone(),
                    name: name.clone(),
                    args: args.clone(),
                })
                .await;

                // Approval: ask tool for its level; unknown tools are Dangerous.
                let level = self
                    .dispatcher
                    .get(&name)
                    .map(|t| t.approval_level(&args))
                    .unwrap_or(ApprovalLevel::Dangerous);

                if let Err(e) = membrane.check(id.clone(), &name, &args, level).await {
                    self.emit(AgentEvent::Failed(e.to_string())).await;
                    return Err(e);
                }

                let tool_results = self
                    .dispatcher
                    .dispatch_sequential(vec![ToolCall {
                        name: name.clone(),
                        args,
                    }])
                    .await?;

                let result = tool_results
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| crate::tools::ToolResult::err("dispatcher returned no result"));

                self.emit(AgentEvent::ToolResult {
                    id: id.clone(),
                    content: result.content.clone(),
                    is_error: result.is_error,
                })
                .await;

                results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": id,
                    "content": result.content,
                    "is_error": result.is_error,
                }));

                // CallingTool → Thinking: ready for next tool or next LLM call.
                state = state.transition(AgentState::Thinking)?;
            }

            let tool_result_msg = serde_json::Value::Array(results);
            messages.push(Message::user(tool_result_msg.clone()));
            self.persist(session_id, "tool_result", &tool_result_msg.to_string())
                .await;
            // Loop continues: state is Thinking, ready for next LLM call.
        }
    }

    /// Fire-and-forget event emission. Drops the event silently if nobody is listening.
    async fn emit(&self, event: AgentEvent) {
        if self.events.send(event).await.is_err() {
            tracing::warn!("event channel closed — dropping event");
        }
    }

    async fn persist(&self, session_id: &str, role: &str, content: &str) {
        if let Some(db) = &self.db {
            db.save_message(session_id, role, content)
                .await
                .unwrap_or_else(|e| tracing::warn!("failed to persist message: {e}"));
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::{future::Future, pin::Pin, sync::Arc};

    use serde_json::Value;
    use tokio::sync::mpsc;

    use talon_llm::{ContentBlock, LlmError, LlmProvider, LlmResponse, Message, MockProvider};

    use crate::{
        approval::ApprovalLevel,
        events::AgentEvent,
        tools::{Tool, ToolContext, ToolResult, dispatcher::ToolDispatcher},
    };

    use super::*;

    // ── Minimal Tool impl for tests ───────────────────────────────────────────

    struct EchoTool;
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn schema(&self) -> Value {
            json!({"name": "echo"})
        }
        fn approval_level(&self, _args: &Value) -> ApprovalLevel {
            ApprovalLevel::Safe
        }
        fn execute(
            &self,
            args: Value,
            _ctx: ToolContext,
        ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
            let msg = args["message"].as_str().unwrap_or("").to_string();
            Box::pin(async move { ToolResult::ok(msg) })
        }
    }

    fn make_dispatcher() -> ToolDispatcher {
        let mut d = ToolDispatcher::new();
        d.register(Arc::new(EchoTool));
        d
    }

    fn make_channel() -> (mpsc::Sender<AgentEvent>, mpsc::Receiver<AgentEvent>) {
        mpsc::channel(64)
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn agent_run_end_turn_emits_completed() {
        let (tx, mut rx) = make_channel();
        let provider = Arc::new(MockProvider::text("Hello!", "end_turn"));
        let mut agent = Agent::new(provider, make_dispatcher(), tx);

        agent.run("sess-1", "hello".to_string()).await.expect("run");

        let events: Vec<_> = {
            let mut v = Vec::new();
            while let Ok(e) = rx.try_recv() {
                v.push(e);
            }
            v
        };
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Started)));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Completed)));
    }

    #[tokio::test]
    async fn agent_run_tool_call_dispatches_and_completes() {
        let (tx, mut rx) = make_channel();
        let provider = Arc::new(MockProvider::new(vec![
            LlmResponse {
                content: vec![ContentBlock::ToolUse {
                    id: "t1".to_string(),
                    name: "echo".to_string(),
                    input: json!({"message": "hi"}),
                }],
                stop_reason: "tool_use".to_string(),
            },
            LlmResponse {
                content: vec![ContentBlock::Text { text: "done".to_string() }],
                stop_reason: "end_turn".to_string(),
            },
        ]));

        let mut agent = Agent::new(provider, make_dispatcher(), tx);
        agent.run("sess-2", "use echo".to_string()).await.expect("run");

        let events: Vec<_> = {
            let mut v = Vec::new();
            while let Ok(e) = rx.try_recv() {
                v.push(e);
            }
            v
        };
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolCalled { .. })));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolResult { .. })));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Completed)));
    }

    #[tokio::test]
    async fn agent_run_persists_to_db() {
        let (tx, _rx) = make_channel();
        let provider = Arc::new(MockProvider::text("stored", "end_turn"));
        let db = Arc::new(talon_memory::Database::open(":memory:").expect("db"));
        db.init_schema().await.expect("schema");

        let mut agent =
            Agent::new(provider, make_dispatcher(), tx).with_db(Arc::clone(&db));
        agent.run("sess-3", "save me".to_string()).await.expect("run");

        let count: i64 = db
            .pool()
            .get()
            .await
            .expect("pool")
            .interact(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM messages WHERE session_id='sess-3'",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .expect("interact")
            .expect("count");
        assert!(count >= 2, "expected at least user + assistant messages, got {count}");
    }

    #[tokio::test]
    async fn agent_run_llm_error_returns_err() {
        struct FailProvider;
        impl LlmProvider for FailProvider {
            fn complete<'a>(
                &'a self,
                _messages: &'a [Message],
                _tools: &'a [Value],
            ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, LlmError>> + Send + 'a>> {
                Box::pin(async { Err(LlmError::AuthFailed) })
            }
        }

        let (tx, _rx) = make_channel();
        let mut agent = Agent::new(Arc::new(FailProvider), make_dispatcher(), tx);
        let result = agent.run("sess-4", "fail".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn agent_run_unknown_tool_stops_with_error() {
        let (tx, _rx) = make_channel();
        let provider = Arc::new(MockProvider::new(vec![LlmResponse {
            content: vec![ContentBlock::ToolUse {
                id: "t1".to_string(),
                name: "ghost_tool".to_string(),
                input: json!({}),
            }],
            stop_reason: "tool_use".to_string(),
        }]));
        // No tools registered — unknown tool defaults to Dangerous.
        // With no receiver to approve, the membrane sends to channel, nobody replies on oneshot.
        // The Dangerous path requires a gateway to respond. In this test we abort by
        // dropping the rx so the membrane gets a closed channel on send.
        drop(tx.clone()); // deliberately close the channel
        // Re-create with a fresh pair but no handler for ApprovalRequested.
        let (tx2, _rx2) = mpsc::channel(1);
        let mut agent = Agent::new(provider, ToolDispatcher::new(), tx2);
        // Drop _rx2 after one event fills the buffer so ApprovalRequested send fails.
        drop(_rx2);
        let result = agent.run("sess-5", "ghost".to_string()).await;
        assert!(result.is_err());
    }
}
