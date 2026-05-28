use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::error::CoreError;

use super::{Tool, ToolContext, ToolResult};

/// Registry + dispatcher for all built-in and plugin tools.
///
/// Sequential dispatch is the default (safe, ordered output, easier to debug).
/// Parallel dispatch is opt-in via `ToolContext::allow_parallel` — cap 4 concurrent tasks.
pub struct ToolDispatcher {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolDispatcher {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Tool schemas in the format the LLM API expects.
    pub fn schemas(&self) -> Vec<serde_json::Value> {
        self.tools.values().map(|t| t.schema()).collect()
    }

    /// Dispatch a list of calls sequentially (default).
    /// Returns results in the same order as `calls`.
    pub async fn dispatch_sequential(
        &self,
        calls: Vec<ToolCall>,
    ) -> Result<Vec<ToolResult>, CoreError> {
        let mut results = Vec::with_capacity(calls.len());
        for call in calls {
            let tool = self.tools.get(&call.name).ok_or_else(|| CoreError::Tool {
                tool: call.name.clone(),
                message: "tool not found".to_string(),
            })?;
            let result = tool.execute(call.args, ToolContext::default()).await;
            results.push(result);
        }
        Ok(results)
    }

    /// Dispatch a list of calls in parallel (opt-in).
    /// Semaphore caps concurrency at 4. Results are returned in completion order.
    pub async fn dispatch_parallel(
        &self,
        calls: Vec<ToolCall>,
    ) -> Result<Vec<ToolResult>, CoreError> {
        let sem = Arc::new(Semaphore::new(4));
        let mut set: JoinSet<Result<ToolResult, CoreError>> = JoinSet::new();

        for call in calls {
            let tool = self
                .tools
                .get(&call.name)
                .cloned()
                .ok_or_else(|| CoreError::Tool {
                    tool: call.name.clone(),
                    message: "tool not found".to_string(),
                })?;
            let sem = Arc::clone(&sem);
            set.spawn(async move {
                let _permit = sem
                    .acquire_owned()
                    .await
                    .map_err(|_| CoreError::InvalidState("semaphore closed".to_string()))?;
                Ok(tool
                    .execute(
                        call.args,
                        ToolContext {
                            allow_parallel: true,
                        },
                    )
                    .await)
            });
        }

        let mut results = Vec::new();
        while let Some(res) = set.join_next().await {
            let inner = res.map_err(|e| CoreError::InvalidState(e.to_string()))??;
            results.push(inner);
        }
        Ok(results)
    }
}

impl Default for ToolDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

/// A single tool invocation request.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub args: serde_json::Value,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use serde_json::Value;

    use crate::approval::ApprovalLevel;

    use super::*;

    struct EchoTool;
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn schema(&self) -> Value {
            serde_json::json!({"name": "echo"})
        }
        fn approval_level(&self, _args: &Value) -> ApprovalLevel {
            ApprovalLevel::Safe
        }
        fn execute(
            &self,
            args: Value,
            _ctx: ToolContext,
        ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
            let msg = args["msg"].as_str().unwrap_or("").to_string();
            Box::pin(async move { ToolResult::ok(msg) })
        }
    }

    struct FailTool;
    impl Tool for FailTool {
        fn name(&self) -> &str {
            "fail"
        }
        fn schema(&self) -> Value {
            serde_json::json!({"name": "fail"})
        }
        fn approval_level(&self, _args: &Value) -> ApprovalLevel {
            ApprovalLevel::Safe
        }
        fn execute(
            &self,
            _args: Value,
            _ctx: ToolContext,
        ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
            Box::pin(async { ToolResult::err("boom") })
        }
    }

    fn make_dispatcher() -> ToolDispatcher {
        let mut d = ToolDispatcher::new();
        d.register(Arc::new(EchoTool));
        d.register(Arc::new(FailTool));
        d
    }

    #[test]
    fn register_and_get_tool() {
        let d = make_dispatcher();
        assert!(d.get("echo").is_some());
        assert!(d.get("unknown").is_none());
    }

    #[test]
    fn schemas_returns_one_per_tool() {
        let d = make_dispatcher();
        assert_eq!(d.schemas().len(), 2);
    }

    #[test]
    fn default_creates_empty_dispatcher() {
        let d = ToolDispatcher::default();
        assert!(d.schemas().is_empty());
    }

    #[tokio::test]
    async fn sequential_dispatch_returns_results_in_order() {
        let d = make_dispatcher();
        let calls = vec![
            ToolCall {
                name: "echo".to_string(),
                args: serde_json::json!({"msg": "a"}),
            },
            ToolCall {
                name: "echo".to_string(),
                args: serde_json::json!({"msg": "b"}),
            },
        ];
        let results = d.dispatch_sequential(calls).await.expect("dispatch");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].content, "a");
        assert_eq!(results[1].content, "b");
    }

    #[tokio::test]
    async fn sequential_dispatch_unknown_tool_returns_error() {
        let d = make_dispatcher();
        let calls = vec![ToolCall {
            name: "nonexistent".to_string(),
            args: serde_json::json!({}),
        }];
        let err = d.dispatch_sequential(calls).await.unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }

    #[tokio::test]
    async fn sequential_dispatch_propagates_tool_error_result() {
        let d = make_dispatcher();
        let calls = vec![ToolCall {
            name: "fail".to_string(),
            args: serde_json::json!({}),
        }];
        let results = d.dispatch_sequential(calls).await.expect("dispatch ok");
        assert!(results[0].is_error);
        assert_eq!(results[0].content, "boom");
    }

    #[tokio::test]
    async fn parallel_dispatch_runs_all_calls() {
        let d = make_dispatcher();
        let calls = vec![
            ToolCall {
                name: "echo".to_string(),
                args: serde_json::json!({"msg": "x"}),
            },
            ToolCall {
                name: "echo".to_string(),
                args: serde_json::json!({"msg": "y"}),
            },
            ToolCall {
                name: "echo".to_string(),
                args: serde_json::json!({"msg": "z"}),
            },
        ];
        let results = d.dispatch_parallel(calls).await.expect("dispatch");
        assert_eq!(results.len(), 3);
        let contents: std::collections::HashSet<_> =
            results.iter().map(|r| r.content.as_str()).collect();
        assert!(contents.contains("x"));
        assert!(contents.contains("y"));
        assert!(contents.contains("z"));
    }

    #[tokio::test]
    async fn parallel_dispatch_unknown_tool_returns_error() {
        let d = make_dispatcher();
        let calls = vec![ToolCall {
            name: "ghost".to_string(),
            args: serde_json::json!({}),
        }];
        let err = d.dispatch_parallel(calls).await.unwrap_err();
        assert!(err.to_string().contains("ghost"));
    }

    #[tokio::test]
    async fn sequential_dispatch_empty_returns_empty() {
        let d = make_dispatcher();
        let results = d.dispatch_sequential(vec![]).await.expect("empty ok");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn parallel_dispatch_empty_returns_empty() {
        let d = make_dispatcher();
        let results = d.dispatch_parallel(vec![]).await.expect("empty ok");
        assert!(results.is_empty());
    }
}
