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
    pub async fn run(&mut self, session_id: &str, user_message: String) -> Result<(), CoreError> {
        let membrane = ApprovalMembrane::new(self.events.clone());
        let tool_schemas = self.dispatcher.schemas();
        let mut state = AgentState::Idle;

        self.emit(AgentEvent::Started).await;
        self.persist(session_id, "user", &user_message).await;

        // LTM recall: surface durable memories relevant to this turn and fold them
        // into the system block so they cross session boundaries.
        let recalled = self.recall_memories(&user_message).await;

        // Every conversation opens with Talon's baseline system message (identity +
        // memory location). Anthropic hoists it to the top-level `system` field;
        // OpenAI-compatible providers accept the system role inline.
        let mut system_text = crate::system_prompt::baseline_system_prompt();
        if !recalled.is_empty() {
            system_text.push_str("\n\n## Remembered\n");
            for memory in &recalled {
                system_text.push_str("- ");
                system_text.push_str(memory);
                system_text.push('\n');
            }
        }

        // Accumulate a plain-text transcript for end-of-turn fact extraction.
        let mut transcript = format!("User: {user_message}\n");

        let mut messages = vec![Message::system(system_text), Message::user(user_message)];

        // Initial: Idle → Thinking before the first LLM call.
        state = state.transition(AgentState::Thinking)?;

        loop {
            self.emit(AgentEvent::LlmRequest).await;

            let response = tokio::time::timeout(
                Duration::from_secs(LLM_TIMEOUT_SECS),
                self.provider.complete(&messages, &tool_schemas),
            )
            .await
            .map_err(|_| CoreError::Timeout {
                secs: LLM_TIMEOUT_SECS,
            })?
            .map_err(|e| CoreError::Llm(e.to_string()))?;

            self.emit(AgentEvent::LlmResponse).await;

            // Emit text content so gateways can display the assistant response.
            for block in &response.content {
                if let ContentBlock::Text { text } = block {
                    transcript.push_str("Assistant: ");
                    transcript.push_str(text);
                    transcript.push('\n');
                    self.emit(AgentEvent::Text {
                        content: text.clone(),
                    })
                    .await;
                }
            }

            let assistant_json = serde_json::to_value(&response.content)
                .map_err(|e| CoreError::InvalidState(e.to_string()))?;
            messages.push(Message::assistant(assistant_json.clone()));
            self.persist(session_id, "assistant", &assistant_json.to_string())
                .await;

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

            // Drive the loop from tool_calls presence, not stop_reason.
            // GitHub Copilot (OpenAI-compat) returns stop_reason="end_turn" even when
            // tool_calls are present — stop_reason is unreliable across providers.
            // max_iterations is the backstop against infinite loops.
            if tool_calls.is_empty() {
                // Session turn complete: extract durable facts and promote them
                // to long-term memory before signalling completion.
                self.extract_and_promote(&transcript).await;
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

                let result = tool_results.into_iter().next().unwrap_or_else(|| {
                    crate::tools::ToolResult::err("dispatcher returned no result")
                });

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

    /// FTS5/BM25 recall of durable memories relevant to the current turn.
    /// Returns memory contents (best-ranked first); empty when there's no DB,
    /// no usable query tokens, or no hits. Recall is best-effort — failures are
    /// logged, never fatal to the turn.
    async fn recall_memories(&self, query: &str) -> Vec<String> {
        const RECALL_LIMIT: usize = 5;
        let Some(db) = &self.db else {
            return Vec::new();
        };
        let Some(fts) = fts5_or_query(query) else {
            return Vec::new();
        };
        let store = talon_memory::LtmStore::new(db.as_ref().clone());
        match store.search_text(&fts, RECALL_LIMIT).await {
            Ok(hits) => hits.into_iter().map(|m| m.content).collect(),
            Err(e) => {
                tracing::warn!("ltm recall failed: {e}");
                Vec::new()
            }
        }
    }

    /// End-of-turn fact extraction: run the transcript through the LLM-backed
    /// `FactExtractor`, then promote durable facts into the `memories` table.
    /// Best-effort — extraction/promotion failures are logged, not propagated.
    async fn extract_and_promote(&self, transcript: &str) {
        const MIN_IMPORTANCE: u8 = 2;
        let Some(db) = &self.db else {
            return;
        };
        let store = talon_memory::LtmStore::new(db.as_ref().clone());
        let completer = crate::memory_bridge::LlmFactCompleter::new(Arc::clone(&self.provider));
        let facts = match talon_memory::FactExtractor::new()
            .extract(transcript, &completer)
            .await
        {
            Ok(facts) => facts,
            Err(e) => {
                tracing::warn!("ltm fact extraction failed: {e}");
                return;
            }
        };
        if facts.is_empty() {
            return;
        }
        let promoter = talon_memory::Promoter::with_min_importance(MIN_IMPORTANCE);
        if let Err(e) = promoter
            .promote(facts, &store, &crate::memory_bridge::ZeroEmbedder)
            .await
        {
            tracing::warn!("ltm promotion failed: {e}");
        }
    }
}

/// Turn arbitrary user text into a safe FTS5 OR-of-phrases query. Each
/// alphanumeric token is double-quoted so FTS5 operators or punctuation in user
/// input can't break the `MATCH` expression. Returns `None` when no usable
/// tokens remain.
fn fts5_or_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"", t.to_lowercase()))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
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

    #[test]
    fn fts5_or_query_quotes_tokens_and_drops_punctuation() {
        assert_eq!(
            fts5_or_query("dark mode?"),
            Some("\"dark\" OR \"mode\"".to_string())
        );
        // Bare FTS5 operators in user input become quoted phrases, not operators.
        assert_eq!(
            fts5_or_query("a OR b"),
            Some("\"a\" OR \"or\" OR \"b\"".to_string())
        );
        assert_eq!(fts5_or_query("   ?! "), None);
    }

    #[tokio::test]
    async fn recall_memories_without_db_is_empty() {
        let (tx, _rx) = make_channel();
        let provider = Arc::new(MockProvider::text("x", "end_turn"));
        let agent = Agent::new(provider, make_dispatcher(), tx);
        assert!(agent.recall_memories("anything at all").await.is_empty());
    }

    #[tokio::test]
    async fn extract_and_promote_without_db_is_noop() {
        // No DB configured → extraction must not touch the provider or panic.
        let (tx, _rx) = make_channel();
        let provider = Arc::new(MockProvider::new(vec![]));
        let agent = Agent::new(provider, make_dispatcher(), tx);
        // Returns cleanly despite the empty (would-be-exhausted) provider queue.
        agent.extract_and_promote("User: hi\n").await;
    }

    #[tokio::test]
    async fn agent_extracts_and_recalls_fact_across_sessions() {
        // Session 1: a normal turn (text, no tools) followed by the extraction
        // call that returns one durable fact as JSON.
        let provider = Arc::new(MockProvider::new(vec![
            LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "Noted.".to_string(),
                }],
                stop_reason: "end_turn".to_string(),
            },
            LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "[{\"content\":\"User prefers dark mode\",\
                           \"category\":\"user_preference\",\"importance\":4}]"
                        .to_string(),
                }],
                stop_reason: "end_turn".to_string(),
            },
        ]));
        let db = Arc::new(talon_memory::Database::open(":memory:").expect("db"));
        db.init_schema().await.expect("schema");

        let (tx, _rx) = make_channel();
        let mut agent = Agent::new(provider, make_dispatcher(), tx).with_db(Arc::clone(&db));
        agent
            .run("s1", "remember that I prefer dark mode".to_string())
            .await
            .expect("session 1 run");

        // The fact was extracted and promoted into the memories table.
        let count: i64 = db
            .pool()
            .get()
            .await
            .expect("pool")
            .interact(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM memories WHERE content LIKE '%dark mode%'",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .expect("interact")
            .expect("query");
        assert_eq!(count, 1, "fact should be promoted to long-term memory");

        // Session 2: a fresh agent over the same DB recalls the fact via FTS5,
        // even though the query word "preferences" only matches "prefers" through
        // the porter stemmer.
        let (tx2, _rx2) = make_channel();
        let provider2 = Arc::new(MockProvider::text("ok", "end_turn"));
        let agent2 = Agent::new(provider2, make_dispatcher(), tx2).with_db(Arc::clone(&db));
        let recalled = agent2
            .recall_memories("what do you know about my preferences?")
            .await;
        assert!(
            recalled.iter().any(|m| m.contains("dark mode")),
            "session 2 should recall the dark-mode preference, got: {recalled:?}"
        );
    }

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
                content: vec![ContentBlock::Text {
                    text: "done".to_string(),
                }],
                stop_reason: "end_turn".to_string(),
            },
        ]));

        let mut agent = Agent::new(provider, make_dispatcher(), tx);
        agent
            .run("sess-2", "use echo".to_string())
            .await
            .expect("run");

        let events: Vec<_> = {
            let mut v = Vec::new();
            while let Ok(e) = rx.try_recv() {
                v.push(e);
            }
            v
        };
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolCalled { .. }))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolResult { .. }))
        );
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Completed)));
    }

    #[tokio::test]
    async fn agent_run_persists_to_db() {
        let (tx, _rx) = make_channel();
        let provider = Arc::new(MockProvider::text("stored", "end_turn"));
        let db = Arc::new(talon_memory::Database::open(":memory:").expect("db"));
        db.init_schema().await.expect("schema");

        let mut agent = Agent::new(provider, make_dispatcher(), tx).with_db(Arc::clone(&db));
        agent
            .run("sess-3", "save me".to_string())
            .await
            .expect("run");

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
        assert!(
            count >= 2,
            "expected at least user + assistant messages, got {count}"
        );
    }

    #[tokio::test]
    async fn agent_run_llm_error_returns_err() {
        struct FailProvider;
        impl LlmProvider for FailProvider {
            fn complete<'a>(
                &'a self,
                _messages: &'a [Message],
                _tools: &'a [Value],
            ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, LlmError>> + Send + 'a>>
            {
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
        let provider = Arc::new(MockProvider::new(vec![LlmResponse {
            content: vec![ContentBlock::ToolUse {
                id: "t1".to_string(),
                name: "ghost_tool".to_string(),
                input: json!({}),
            }],
            stop_reason: "tool_use".to_string(),
        }]));
        // No tools registered — unknown tool defaults to Dangerous.
        // Drop the receiver so the membrane's send on the approval event fails immediately.
        let (tx, rx) = mpsc::channel::<AgentEvent>(1);
        drop(rx);
        let mut agent = Agent::new(provider, ToolDispatcher::new(), tx);
        let result = agent.run("sess-5", "ghost".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn agent_run_emits_llm_request_and_response_events() {
        let (tx, mut rx) = make_channel();
        let provider = Arc::new(MockProvider::text("answer", "end_turn"));
        let mut agent = Agent::new(provider, make_dispatcher(), tx);
        agent
            .run("sess-6", "question".to_string())
            .await
            .expect("run");

        let events: Vec<_> = {
            let mut v = Vec::new();
            while let Ok(e) = rx.try_recv() {
                v.push(e);
            }
            v
        };
        assert!(events.iter().any(|e| matches!(e, AgentEvent::LlmRequest)));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::LlmResponse)));
    }

    #[tokio::test]
    async fn agent_run_two_tools_in_one_response() {
        let (tx, mut rx) = make_channel();
        let provider = Arc::new(MockProvider::new(vec![
            LlmResponse {
                content: vec![
                    ContentBlock::ToolUse {
                        id: "t1".to_string(),
                        name: "echo".to_string(),
                        input: json!({"message": "first"}),
                    },
                    ContentBlock::ToolUse {
                        id: "t2".to_string(),
                        name: "echo".to_string(),
                        input: json!({"message": "second"}),
                    },
                ],
                stop_reason: "tool_use".to_string(),
            },
            LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "both done".to_string(),
                }],
                stop_reason: "end_turn".to_string(),
            },
        ]));

        let mut agent = Agent::new(provider, make_dispatcher(), tx);
        agent
            .run("sess-7", "two tools".to_string())
            .await
            .expect("run");

        let events: Vec<_> = {
            let mut v = Vec::new();
            while let Ok(e) = rx.try_recv() {
                v.push(e);
            }
            v
        };
        let tool_called_count = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::ToolCalled { .. }))
            .count();
        let tool_result_count = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::ToolResult { .. }))
            .count();
        assert_eq!(
            tool_called_count, 2,
            "expected 2 ToolCalled events, got {tool_called_count}"
        );
        assert_eq!(
            tool_result_count, 2,
            "expected 2 ToolResult events, got {tool_result_count}"
        );
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Completed)));
    }

    /// Regression: GitHub Copilot (OpenAI-compat) returns stop_reason="end_turn" even when
    /// tool_calls are present. The loop must drive from tool_calls presence, not stop_reason.
    #[tokio::test]
    async fn agent_run_tool_executes_when_stop_reason_is_end_turn() {
        let (tx, mut rx) = make_channel();
        let provider = Arc::new(MockProvider::new(vec![
            LlmResponse {
                content: vec![ContentBlock::ToolUse {
                    id: "t1".to_string(),
                    name: "echo".to_string(),
                    input: json!({"message": "smoke-test-ok"}),
                }],
                stop_reason: "end_turn".to_string(), // Copilot sends this even with tool_calls
            },
            LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "smoke-test-ok".to_string(),
                }],
                stop_reason: "end_turn".to_string(),
            },
        ]));

        let mut agent = Agent::new(provider, make_dispatcher(), tx);
        agent
            .run("sess-copilot", "echo smoke-test-ok".to_string())
            .await
            .expect("run");

        let events: Vec<_> = {
            let mut v = Vec::new();
            while let Ok(e) = rx.try_recv() {
                v.push(e);
            }
            v
        };
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolCalled { .. })),
            "ToolCalled must fire even when stop_reason=end_turn — loop must be driven by tool_calls"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolResult { .. })),
            "ToolResult must be emitted after tool execution"
        );
    }

    #[tokio::test]
    async fn agent_run_prepends_baseline_system_message() {
        use std::sync::Mutex;

        // Provider that records the messages it was handed on the first call.
        struct CaptureProvider {
            seen: Arc<Mutex<Vec<Message>>>,
        }
        impl LlmProvider for CaptureProvider {
            fn complete<'a>(
                &'a self,
                messages: &'a [Message],
                _tools: &'a [Value],
            ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, LlmError>> + Send + 'a>>
            {
                *self.seen.lock().expect("lock") = messages.to_vec();
                Box::pin(async {
                    Ok(LlmResponse {
                        content: vec![ContentBlock::Text {
                            text: "ok".to_string(),
                        }],
                        stop_reason: "end_turn".to_string(),
                    })
                })
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(CaptureProvider {
            seen: Arc::clone(&seen),
        });
        let (tx, _rx) = make_channel();
        let mut agent = Agent::new(provider, make_dispatcher(), tx);
        agent.run("sess-sys", "hi".to_string()).await.expect("run");

        let messages = seen.lock().expect("lock");
        assert_eq!(messages[0].role, "system", "system message must come first");
        assert!(
            messages[0]
                .content
                .as_str()
                .expect("system content is text")
                .contains("SQLite"),
            "baseline must tell Talon its memory is on SQLite"
        );
        assert_eq!(messages[1].role, "user");
    }

    #[tokio::test]
    async fn agent_run_no_text_in_end_turn_still_completes() {
        let (tx, mut rx) = make_channel();
        // Response with no text blocks — just stop_reason "end_turn"
        let provider = Arc::new(MockProvider::new(vec![LlmResponse {
            content: vec![],
            stop_reason: "end_turn".to_string(),
        }]));
        let mut agent = Agent::new(provider, make_dispatcher(), tx);
        agent.run("sess-8", "quiet".to_string()).await.expect("run");

        let events: Vec<_> = {
            let mut v = Vec::new();
            while let Ok(e) = rx.try_recv() {
                v.push(e);
            }
            v
        };
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Completed)));
    }
}
