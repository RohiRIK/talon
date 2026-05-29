//! Per-class tool timeouts (task 5.9).
//!
//! Thin wiring over [`crate::timeout::TimeoutWrapper`] that pins the wall-clock
//! budget for each tool class and hands back an `Arc<dyn Tool>` ready to
//! register with the dispatcher.

use std::sync::Arc;
use std::time::Duration;

use talon_core::tools::Tool;

use crate::timeout::TimeoutWrapper;

/// Web tools (search, extract).
pub const WEB_TIMEOUT_SECS: u64 = 30;
/// MCP tool calls.
pub const MCP_TIMEOUT_SECS: u64 = 30;
/// Subprocess plugins.
pub const SUBPROCESS_TIMEOUT_SECS: u64 = 30;
/// Browser navigation (heavier; renders JS).
pub const BROWSER_TIMEOUT_SECS: u64 = 60;

/// Wrap a concrete tool with a wall-clock timeout and erase it to
/// `Arc<dyn Tool>` for registration.
pub fn with_timeout<T: Tool + 'static>(tool: T, secs: u64) -> Arc<dyn Tool> {
    Arc::new(TimeoutWrapper::new(tool, Duration::from_secs(secs)))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::future::Future;
    use std::pin::Pin;
    use talon_core::approval::ApprovalLevel;
    use talon_core::tools::{ToolContext, ToolResult};

    struct SlowTool;
    impl Tool for SlowTool {
        fn name(&self) -> &str {
            "slow"
        }
        fn schema(&self) -> Value {
            Value::Null
        }
        fn approval_level(&self, _: &Value) -> ApprovalLevel {
            ApprovalLevel::Safe
        }
        fn execute(
            &self,
            _: Value,
            _: ToolContext,
        ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                ToolResult::ok("done")
            })
        }
    }

    #[test]
    fn class_budgets_are_correct() {
        assert_eq!(WEB_TIMEOUT_SECS, 30);
        assert_eq!(MCP_TIMEOUT_SECS, 30);
        assert_eq!(SUBPROCESS_TIMEOUT_SECS, 30);
        assert_eq!(BROWSER_TIMEOUT_SECS, 60);
    }

    #[tokio::test]
    async fn over_budget_tool_times_out() {
        // 0s budget → the 30s-sleeping tool is cut off immediately.
        let tool = with_timeout(SlowTool, 0);
        let r = tool.execute(Value::Null, ToolContext::default()).await;
        assert!(r.is_error);
        assert!(r.content.contains("timed out"), "got: {}", r.content);
    }
}
