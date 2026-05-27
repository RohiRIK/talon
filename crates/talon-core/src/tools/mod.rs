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
    fn tool_context_default_is_sequential() {
        let ctx = ToolContext::default();
        assert!(!ctx.allow_parallel);
    }
}
