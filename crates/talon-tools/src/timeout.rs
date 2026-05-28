use std::{future::Future, pin::Pin, time::Duration};

use serde_json::Value;
use talon_core::approval::ApprovalLevel;
use talon_core::tools::{Tool, ToolContext, ToolResult};

/// Wraps any `Tool` with a wall-clock timeout.
/// If the inner tool takes longer than `duration`, returns an error result.
pub struct TimeoutWrapper<T: Tool> {
    inner: T,
    duration: Duration,
}

impl<T: Tool> TimeoutWrapper<T> {
    pub fn new(inner: T, duration: Duration) -> Self {
        Self { inner, duration }
    }
}

impl<T: Tool + 'static> Tool for TimeoutWrapper<T> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn schema(&self) -> Value {
        self.inner.schema()
    }

    fn approval_level(&self, args: &Value) -> ApprovalLevel {
        self.inner.approval_level(args)
    }

    fn execute(
        &self,
        args: Value,
        ctx: ToolContext,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let duration = self.duration;
        Box::pin(async move {
            match tokio::time::timeout(duration, self.inner.execute(args, ctx)).await {
                Ok(result) => result,
                Err(_) => ToolResult::err(format!(
                    "tool '{}' timed out after {}s",
                    self.inner.name(),
                    duration.as_secs()
                )),
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use talon_core::approval::ApprovalLevel;
    use talon_core::tools::Tool;

    struct InstantTool;
    impl Tool for InstantTool {
        fn name(&self) -> &str {
            "instant"
        }
        fn schema(&self) -> Value {
            serde_json::json!({"name": "instant"})
        }
        fn approval_level(&self, _: &Value) -> ApprovalLevel {
            ApprovalLevel::Safe
        }
        fn execute(
            &self,
            args: Value,
            _: ToolContext,
        ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
            let msg = args["msg"].as_str().unwrap_or("ok").to_string();
            Box::pin(async move { ToolResult::ok(msg) })
        }
    }

    struct SlowTool;
    impl Tool for SlowTool {
        fn name(&self) -> &str {
            "slow"
        }
        fn schema(&self) -> Value {
            serde_json::json!({"name": "slow"})
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
                tokio::time::sleep(Duration::from_secs(60)).await;
                ToolResult::ok("never")
            })
        }
    }

    #[tokio::test]
    async fn fast_tool_completes_within_timeout() {
        let w = TimeoutWrapper::new(InstantTool, Duration::from_secs(5));
        let r = w
            .execute(serde_json::json!({"msg": "hi"}), ToolContext::default())
            .await;
        assert!(!r.is_error);
        assert_eq!(r.content, "hi");
    }

    #[tokio::test]
    async fn slow_tool_returns_timeout_error() {
        let w = TimeoutWrapper::new(SlowTool, Duration::from_millis(50));
        let r = w.execute(Value::Null, ToolContext::default()).await;
        assert!(r.is_error);
        assert!(r.content.contains("timed out"));
        assert!(r.content.contains("slow"));
    }

    #[test]
    fn wrapper_delegates_name() {
        let w = TimeoutWrapper::new(InstantTool, Duration::from_secs(1));
        assert_eq!(w.name(), "instant");
    }

    #[test]
    fn wrapper_delegates_schema() {
        let w = TimeoutWrapper::new(InstantTool, Duration::from_secs(1));
        assert_eq!(w.schema()["name"], "instant");
    }

    #[test]
    fn wrapper_delegates_approval_level() {
        let w = TimeoutWrapper::new(InstantTool, Duration::from_secs(1));
        assert_eq!(w.approval_level(&Value::Null), ApprovalLevel::Safe);
    }

    #[test]
    fn arc_dyn_tool_wraps_timeout_wrapper() {
        let _: Arc<dyn Tool> = Arc::new(TimeoutWrapper::new(InstantTool, Duration::from_secs(5)));
    }
}
