use std::{future::Future, pin::Pin};

use serde_json::{Value, json};
use talon_core::{
    approval::ApprovalLevel,
    tools::{Tool, ToolContext, ToolResult},
};

use super::SandboxBackend;

/// A `Tool` that delegates to any `SandboxBackend`.
/// Always requires explicit user approval (`ApprovalLevel::Dangerous`).
pub struct TerminalTool<B: SandboxBackend> {
    backend: B,
}

impl<B: SandboxBackend> TerminalTool<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }
}

impl<B: SandboxBackend + 'static> Tool for TerminalTool<B> {
    fn name(&self) -> &str {
        "run_command"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "run_command",
            "description": "Execute a shell command in the configured sandbox (Docker or native host). Native mode always tags output with [NATIVE].",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Shell command to run." }
                },
                "required": ["command"]
            }
        })
    }

    fn approval_level(&self, _args: &Value) -> ApprovalLevel {
        ApprovalLevel::Dangerous
    }

    fn execute(
        &self,
        args: Value,
        _ctx: ToolContext,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        Box::pin(async move {
            let cmd = match args["command"].as_str() {
                Some(c) => c.to_string(),
                None => return ToolResult::err("missing required argument: command"),
            };
            self.backend.execute(&cmd).await
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::terminal::NativeBackend;

    #[tokio::test]
    async fn delegates_to_backend() {
        let t = TerminalTool::new(NativeBackend);
        let r = t
            .execute(json!({"command": "echo tool_ok"}), ToolContext::default())
            .await;
        assert!(!r.is_error, "{}", r.content);
        assert!(r.content.contains("tool_ok"));
    }

    #[tokio::test]
    async fn returns_error_for_missing_command() {
        let t = TerminalTool::new(NativeBackend);
        let r = t.execute(json!({}), ToolContext::default()).await;
        assert!(r.is_error);
        assert!(r.content.contains("missing required argument: command"));
    }

    #[test]
    fn name_is_run_command() {
        assert_eq!(TerminalTool::new(NativeBackend).name(), "run_command");
    }

    #[test]
    fn approval_level_is_dangerous() {
        assert_eq!(
            TerminalTool::new(NativeBackend).approval_level(&Value::Null),
            ApprovalLevel::Dangerous
        );
    }
}
