use std::{future::Future, pin::Pin};

use serde_json::Value;

use crate::approval::ApprovalLevel;

/// Type #1 — output of every tool execution.
#[derive(Debug)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn err(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
        }
    }
}

/// Execution context passed to every tool call.
/// Sequential dispatch is the default; parallel is opt-in per PLAN.md.
#[derive(Debug, Default)]
pub struct ToolContext {
    pub allow_parallel: bool,
}

/// Type #2 — the tool interface.
///
/// `execute` returns `Pin<Box<dyn Future>>` rather than `async fn` so the trait
/// is dyn-compatible — required by `Arc<dyn Tool>` (Type #5). See ADR 0007.
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn schema(&self) -> Value;
    /// Approval is computed per-invocation with the actual arguments,
    /// not as a static property of the tool definition.
    fn approval_level(&self, args: &Value) -> ApprovalLevel;
    fn execute(
        &self,
        args: Value,
        ctx: ToolContext,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn tool_result_ok_is_not_error() {
        let r = ToolResult::ok("hello");
        assert!(!r.is_error);
        assert_eq!(r.content, "hello");
    }

    #[test]
    fn tool_result_err_is_error() {
        let r = ToolResult::err("boom");
        assert!(r.is_error);
        assert_eq!(r.content, "boom");
    }

    #[test]
    fn tool_result_ok_accepts_string_ref() {
        let s = "from &str".to_string();
        let r = ToolResult::ok(s.as_str());
        assert_eq!(r.content, "from &str");
    }

    #[test]
    fn tool_result_debug_shows_content() {
        let r = ToolResult::ok("abc");
        assert!(format!("{r:?}").contains("abc"));
    }

    #[test]
    fn tool_context_default_is_sequential() {
        let ctx = ToolContext::default();
        assert!(!ctx.allow_parallel);
    }

    #[test]
    fn tool_context_parallel_can_be_set() {
        let ctx = ToolContext { allow_parallel: true };
        assert!(ctx.allow_parallel);
    }

    /// Type #5 validation — Arc<dyn Tool> must work with the Pin<Box<dyn Future>> pattern.
    /// This is the critical claim from ADR 0007: the trait is dyn-compatible.
    #[tokio::test]
    async fn arc_dyn_tool_dispatches_and_returns_result() {
        struct EchoTool;
        impl Tool for EchoTool {
            fn name(&self) -> &str {
                "echo"
            }
            fn schema(&self) -> Value {
                serde_json::json!({})
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

        let tool: Arc<dyn Tool> = Arc::new(EchoTool);
        assert_eq!(tool.name(), "echo");
        assert_eq!(tool.approval_level(&Value::Null), ApprovalLevel::Safe);

        let result = tool
            .execute(serde_json::json!({"msg": "hello"}), ToolContext::default())
            .await;
        assert!(!result.is_error);
        assert_eq!(result.content, "hello");
    }

    /// Verify two Arc<dyn Tool> references can coexist — tests Arc cloneability.
    #[tokio::test]
    async fn arc_dyn_tool_can_be_cloned_and_shared() {
        struct NoopTool;
        impl Tool for NoopTool {
            fn name(&self) -> &str {
                "noop"
            }
            fn schema(&self) -> Value {
                serde_json::json!({})
            }
            fn approval_level(&self, _args: &Value) -> ApprovalLevel {
                ApprovalLevel::NeedsApproval
            }
            fn execute(
                &self,
                _args: Value,
                _ctx: ToolContext,
            ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
                Box::pin(async { ToolResult::ok("noop") })
            }
        }

        let a: Arc<dyn Tool> = Arc::new(NoopTool);
        let b = Arc::clone(&a);
        assert_eq!(a.name(), b.name());
        let ra = a.execute(Value::Null, ToolContext::default()).await;
        let rb = b.execute(Value::Null, ToolContext::default()).await;
        assert_eq!(ra.content, rb.content);
    }

    /// Dangerous tool returns the correct level — exercises all three ApprovalLevel variants
    /// through the Tool trait boundary, not just the enum directly.
    #[test]
    fn tool_trait_approval_level_all_variants() {
        struct DangerTool;
        impl Tool for DangerTool {
            fn name(&self) -> &str { "danger" }
            fn schema(&self) -> Value { serde_json::json!({}) }
            fn approval_level(&self, _args: &Value) -> ApprovalLevel {
                ApprovalLevel::Dangerous
            }
            fn execute(
                &self,
                _args: Value,
                _ctx: ToolContext,
            ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
                Box::pin(async { ToolResult::err("denied") })
            }
        }
        assert!(matches!(
            DangerTool.approval_level(&Value::Null),
            ApprovalLevel::Dangerous
        ));
    }
}
