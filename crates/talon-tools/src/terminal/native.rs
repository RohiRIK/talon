use std::{future::Future, pin::Pin};

use talon_core::tools::ToolResult;

use super::{SandboxBackend, SandboxMode};

/// Executes commands directly on the host without any isolation.
/// Always requires explicit user approval (ApprovalLevel::Dangerous).
/// Prepends `[NATIVE]` to every result so the LLM and user always know the mode.
pub struct NativeBackend;

impl SandboxBackend for NativeBackend {
    fn mode(&self) -> SandboxMode {
        SandboxMode::Native
    }

    fn execute(&self, cmd: &str) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let cmd = cmd.to_string();
        Box::pin(async move {
            let output = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .output()
                .await;

            match output {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let combined = if stderr.is_empty() {
                        stdout.into_owned()
                    } else {
                        format!("{stdout}{stderr}")
                    };
                    if out.status.success() {
                        ToolResult::ok(format!("[NATIVE] {combined}"))
                    } else {
                        ToolResult::err(format!(
                            "[NATIVE] exit {}: {combined}",
                            out.status.code().unwrap_or(-1)
                        ))
                    }
                }
                Err(e) => ToolResult::err(format!("[NATIVE] failed to spawn: {e}")),
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn echo_command_succeeds_with_native_tag() {
        let b = NativeBackend;
        let r = b.execute("echo hello").await;
        assert!(!r.is_error, "{}", r.content);
        assert!(r.content.starts_with("[NATIVE]"));
        assert!(r.content.contains("hello"));
    }

    #[tokio::test]
    async fn failing_command_returns_error_with_native_tag() {
        let b = NativeBackend;
        let r = b.execute("exit 1").await;
        assert!(r.is_error);
        assert!(r.content.starts_with("[NATIVE]"));
    }

    #[test]
    fn mode_is_native() {
        assert_eq!(NativeBackend.mode(), SandboxMode::Native);
    }
}
