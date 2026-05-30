//! Subprocess plugin: expose an external process as a [`Tool`] via JSON over stdio.
//!
//! This is the plugin entry point that precedes the WASM host (Phase 6). The
//! newline-delimited JSON framing here is intentionally the same shape the MCP
//! stdio transport uses (task 5.5), so the wire handling can be shared later.

use std::process::Stdio;
use std::{future::Future, pin::Pin};

use serde_json::{Value, json};
use talon_core::{
    approval::ApprovalLevel,
    tools::{Tool, ToolContext, ToolResult},
};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// A [`Tool`] backed by an external process.
///
/// Protocol (newline-delimited JSON, one round trip per call):
/// - talon → plugin (stdin):  `{"name": "<tool>", "args": { ... }}\n`
/// - plugin → talon (stdout): `{"content": "<text>", "is_error": <bool>}\n`
///
/// A fresh process is spawned per invocation (stateless). `NeedsApproval` —
/// running an external program is never silently auto-approved.
pub struct SubprocessPlugin {
    name: String,
    description: String,
    input_schema: Value,
    command: String,
    args: Vec<String>,
}

impl SubprocessPlugin {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        command: impl Into<String>,
        args: Vec<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
            command: command.into(),
            args,
        }
    }
}

impl Tool for SubprocessPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn schema(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "input_schema": self.input_schema,
        })
    }

    fn approval_level(&self, _args: &Value) -> ApprovalLevel {
        ApprovalLevel::NeedsApproval
    }

    fn execute(
        &self,
        args: Value,
        _ctx: ToolContext,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let command = self.command.clone();
        let cmd_args = self.args.clone();
        let name = self.name.clone();
        Box::pin(async move {
            let request = json!({ "name": name, "args": args });
            let mut line = match serde_json::to_string(&request) {
                Ok(s) => s,
                Err(e) => return ToolResult::err(format!("failed to serialize request: {e}")),
            };
            line.push('\n');

            let mut child = match Command::new(&command)
                .args(&cmd_args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    return ToolResult::err(format!("failed to spawn plugin '{command}': {e}"));
                }
            };

            // Best-effort: send the request, then drop stdin to signal EOF. A
            // plugin that ignores stdin (and may have already exited) causes a
            // broken pipe on some platforms — that is not a tool failure; the
            // child's stdout and exit status are the source of truth.
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(line.as_bytes()).await;
                let _ = stdin.flush().await;
            }

            let output = match child.wait_with_output().await {
                Ok(o) => o,
                Err(e) => return ToolResult::err(format!("plugin '{command}' failed: {e}")),
            };

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return ToolResult::err(format!(
                    "plugin '{command}' exited with {}: {}",
                    output.status,
                    stderr.trim()
                ));
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            let first = stdout.lines().next().unwrap_or("").trim();
            if first.is_empty() {
                return ToolResult::err(format!("plugin '{command}' produced no output"));
            }

            let resp: Value = match serde_json::from_str(first) {
                Ok(v) => v,
                Err(e) => {
                    return ToolResult::err(format!(
                        "plugin '{command}' returned invalid JSON: {e}"
                    ));
                }
            };

            let content = resp["content"].as_str().unwrap_or("").to_string();
            if resp["is_error"].as_bool().unwrap_or(false) {
                ToolResult::err(content)
            } else {
                ToolResult::ok(content)
            }
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn metadata_needs_approval_and_named() {
        let p = SubprocessPlugin::new("p", "desc", json!({}), "true", vec![]);
        assert_eq!(p.name(), "p");
        assert_eq!(p.approval_level(&Value::Null), ApprovalLevel::NeedsApproval);
        assert_eq!(p.schema()["name"], "p");
        assert_eq!(p.schema()["description"], "desc");
    }

    #[tokio::test]
    async fn missing_binary_is_error() {
        let p = SubprocessPlugin::new("p", "d", json!({}), "talon-nonexistent-binary-zzz", vec![]);
        let r = p.execute(json!({}), ToolContext::default()).await;
        assert!(r.is_error);
        assert!(r.content.contains("failed to spawn"), "got: {}", r.content);
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod unix_tests {
    use super::*;

    /// Build a plugin that runs an inline `sh` script.
    fn sh_plugin(script: &str) -> SubprocessPlugin {
        SubprocessPlugin::new(
            "sh_plugin",
            "test plugin",
            json!({}),
            "sh",
            vec!["-c".to_string(), script.to_string()],
        )
    }

    #[tokio::test]
    async fn round_trips_success() {
        let p = sh_plugin(
            r#"cat >/dev/null; printf '{"content":"hello from plugin","is_error":false}\n'"#,
        );
        let r = p.execute(json!({ "x": 1 }), ToolContext::default()).await;
        assert!(!r.is_error, "got: {}", r.content);
        assert_eq!(r.content, "hello from plugin");
    }

    #[tokio::test]
    async fn plugin_error_response_maps_to_err() {
        let p = sh_plugin(r#"cat >/dev/null; printf '{"content":"boom","is_error":true}\n'"#);
        let r = p.execute(json!({}), ToolContext::default()).await;
        assert!(r.is_error);
        assert_eq!(r.content, "boom");
    }

    #[tokio::test]
    async fn invalid_json_is_error() {
        let p = sh_plugin(r#"cat >/dev/null; printf 'not json at all\n'"#);
        let r = p.execute(json!({}), ToolContext::default()).await;
        assert!(r.is_error);
        assert!(r.content.contains("invalid JSON"), "got: {}", r.content);
    }

    #[tokio::test]
    async fn request_is_delivered_on_stdin() {
        // The plugin only sees the marker if our request reached its stdin.
        let p = sh_plugin(
            r#"req=$(cat); case "$req" in
                 *marker-xyz*) printf '{"content":"saw marker","is_error":false}\n' ;;
                 *) printf '{"content":"no marker","is_error":true}\n' ;;
               esac"#,
        );
        let r = p
            .execute(json!({ "tag": "marker-xyz" }), ToolContext::default())
            .await;
        assert!(!r.is_error, "got: {}", r.content);
        assert_eq!(r.content, "saw marker");
    }

    #[tokio::test]
    async fn nonzero_exit_is_error() {
        let p = sh_plugin(r#"echo "kaboom" >&2; exit 3"#);
        let r = p.execute(json!({}), ToolContext::default()).await;
        assert!(r.is_error);
        assert!(r.content.contains("kaboom"), "got: {}", r.content);
    }
}
