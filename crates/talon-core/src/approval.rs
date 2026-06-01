use serde_json::Value;
use tokio::sync::mpsc;

use talon_memory::GrantedScope;

use crate::error::CoreError;
use crate::events::AgentEvent;

/// Per-invocation approval classification. Computed with actual tool arguments,
/// not as a static property of the tool — prevents tools lying about danger level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalLevel {
    Safe,
    NeedsApproval,
    Dangerous,
}

/// Shell control operators that turn a single allowlisted command into a vector
/// for arbitrary follow-on commands. A Bash grant must never permit these.
const SHELL_CONTROL_OPERATORS: [&str; 7] = [";", "&&", "||", "|", "`", "$(", "\n"];

/// Is a concrete Bash `command` permitted by a job's `bash_patterns` allowlist?
///
/// A pattern matches only when the command *is* the pattern or begins with the
/// pattern followed by a space — `git pull` permits `git pull origin main` but
/// not `git pullx`. Any shell control operator (`;`, `&&`, `|`, backtick, …)
/// rejects outright, so a `git pull` grant can never chain into `rm -rf`.
pub fn bash_command_allowed(command: &str, patterns: &[String]) -> bool {
    let cmd = command.trim();
    if cmd.is_empty() {
        return false;
    }
    if SHELL_CONTROL_OPERATORS.iter().any(|op| cmd.contains(op)) {
        return false;
    }
    patterns.iter().any(|p| {
        let p = p.trim();
        !p.is_empty() && (cmd == p || cmd.starts_with(&format!("{p} ")))
    })
}

/// Does a job's `granted_scope` pre-authorize this tool call for unattended use?
/// The `terminal` tool additionally requires its `command` to match the Bash
/// allowlist; every other tool is permitted by name alone.
pub fn scope_allows(scope: &GrantedScope, tool_name: &str, args: &Value) -> bool {
    if !scope.tools.iter().any(|t| t == tool_name) {
        return false;
    }
    if tool_name == "terminal" {
        let command = args["command"].as_str().unwrap_or_default();
        return bash_command_allowed(command, &scope.bash_patterns);
    }
    true
}

/// The effective approval level for a tool call running **unattended** under a
/// granted scope (SPEC §4.4). `Safe` reads always pass; a `Dangerous` tool is
/// never auto-granted and always escalates; a `NeedsApproval` tool runs only if
/// it is inside the pre-authorized scope, otherwise it escalates.
pub fn effective_unattended_level(
    base: ApprovalLevel,
    scope: &GrantedScope,
    tool_name: &str,
    args: &Value,
) -> ApprovalLevel {
    match base {
        ApprovalLevel::Safe => ApprovalLevel::Safe,
        ApprovalLevel::Dangerous => ApprovalLevel::Dangerous,
        ApprovalLevel::NeedsApproval => {
            if scope_allows(scope, tool_name, args) {
                ApprovalLevel::Safe
            } else {
                ApprovalLevel::Dangerous
            }
        }
    }
}

/// Sits between the agent loop and every tool call.
/// Safe and NeedsApproval pass through immediately.
/// Dangerous emits `AgentEvent::ApprovalRequested` and awaits the gateway's reply.
pub struct ApprovalMembrane {
    events: mpsc::Sender<AgentEvent>,
}

impl ApprovalMembrane {
    pub fn new(events: mpsc::Sender<AgentEvent>) -> Self {
        Self { events }
    }

    pub async fn check(
        &self,
        call_id: String,
        tool_name: &str,
        args: &serde_json::Value,
        level: ApprovalLevel,
    ) -> Result<(), CoreError> {
        match level {
            ApprovalLevel::Safe | ApprovalLevel::NeedsApproval => Ok(()),
            ApprovalLevel::Dangerous => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                self.events
                    .send(AgentEvent::ApprovalRequested {
                        call_id,
                        tool_name: tool_name.to_string(),
                        args: args.clone(),
                        approval_level: level,
                        tx,
                    })
                    .await
                    .map_err(|_| CoreError::InvalidState("event channel closed".to_string()))?;

                let approved = rx
                    .await
                    .map_err(|_| CoreError::InvalidState("approval sender dropped".to_string()))?;

                if approved {
                    Ok(())
                } else {
                    Err(CoreError::ApprovalDenied {
                        tool: tool_name.to_string(),
                    })
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    fn scope(tools: &[&str], bash: &[&str]) -> GrantedScope {
        GrantedScope {
            tools: tools.iter().map(|s| s.to_string()).collect(),
            bash_patterns: bash.iter().map(|s| s.to_string()).collect(),
        }
    }

    // ── bash_command_allowed ────────────────────────────────────────────────

    #[test]
    fn bash_exact_and_prefixed_match() {
        let patterns = vec!["git pull".to_string()];
        assert!(bash_command_allowed("git pull", &patterns));
        assert!(bash_command_allowed("git pull origin main", &patterns));
    }

    #[test]
    fn bash_rejects_non_prefix() {
        let patterns = vec!["git pull".to_string()];
        assert!(!bash_command_allowed("git pullx", &patterns));
        assert!(!bash_command_allowed("rm -rf /", &patterns));
    }

    #[test]
    fn bash_rejects_chaining_into_danger() {
        let patterns = vec!["git pull".to_string()];
        assert!(!bash_command_allowed("git pull; rm -rf /", &patterns));
        assert!(!bash_command_allowed("git pull && rm -rf /", &patterns));
        assert!(!bash_command_allowed("git pull | sh", &patterns));
        assert!(!bash_command_allowed("git pull `whoami`", &patterns));
        assert!(!bash_command_allowed("git pull $(whoami)", &patterns));
    }

    #[test]
    fn bash_empty_command_or_patterns_denied() {
        assert!(!bash_command_allowed("", &["git pull".to_string()]));
        assert!(!bash_command_allowed("git pull", &[]));
    }

    // ── scope_allows ────────────────────────────────────────────────────────

    #[test]
    fn scope_allows_named_tool() {
        let s = scope(&["web_search"], &[]);
        assert!(scope_allows(&s, "web_search", &json!({})));
        assert!(!scope_allows(&s, "send_message", &json!({})));
    }

    #[test]
    fn scope_terminal_requires_bash_match() {
        let s = scope(&["terminal"], &["git pull"]);
        assert!(scope_allows(
            &s,
            "terminal",
            &json!({"command": "git pull"})
        ));
        assert!(!scope_allows(
            &s,
            "terminal",
            &json!({"command": "rm -rf /"})
        ));
        // terminal in tools but no command → denied.
        assert!(!scope_allows(&s, "terminal", &json!({})));
    }

    // ── effective_unattended_level ──────────────────────────────────────────

    #[test]
    fn unattended_safe_stays_safe() {
        let s = scope(&[], &[]);
        assert_eq!(
            effective_unattended_level(ApprovalLevel::Safe, &s, "read_file", &json!({})),
            ApprovalLevel::Safe
        );
    }

    #[test]
    fn unattended_dangerous_always_escalates() {
        // Even if (mistakenly) listed in scope, Dangerous never auto-grants.
        let s = scope(&["terminal"], &["rm -rf /"]);
        assert_eq!(
            effective_unattended_level(
                ApprovalLevel::Dangerous,
                &s,
                "terminal",
                &json!({"command": "rm -rf /"})
            ),
            ApprovalLevel::Dangerous
        );
    }

    #[test]
    fn unattended_needs_approval_in_scope_runs_free() {
        let s = scope(&["send_message"], &[]);
        assert_eq!(
            effective_unattended_level(
                ApprovalLevel::NeedsApproval,
                &s,
                "send_message",
                &json!({})
            ),
            ApprovalLevel::Safe
        );
    }

    #[test]
    fn unattended_needs_approval_out_of_scope_escalates() {
        let s = scope(&["read_file"], &[]);
        assert_eq!(
            effective_unattended_level(
                ApprovalLevel::NeedsApproval,
                &s,
                "send_message",
                &json!({})
            ),
            ApprovalLevel::Dangerous
        );
    }

    #[test]
    fn approval_level_equality() {
        assert_eq!(ApprovalLevel::Safe, ApprovalLevel::Safe);
        assert_ne!(ApprovalLevel::Safe, ApprovalLevel::Dangerous);
    }

    #[test]
    fn approval_level_copy() {
        let a = ApprovalLevel::NeedsApproval;
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn approval_level_debug() {
        assert_eq!(format!("{:?}", ApprovalLevel::Dangerous), "Dangerous");
    }

    #[test]
    fn all_three_variants_are_distinct() {
        assert_ne!(ApprovalLevel::Safe, ApprovalLevel::NeedsApproval);
        assert_ne!(ApprovalLevel::NeedsApproval, ApprovalLevel::Dangerous);
        assert_ne!(ApprovalLevel::Safe, ApprovalLevel::Dangerous);
    }

    #[test]
    fn pattern_match_safe() {
        let level = ApprovalLevel::Safe;
        let auto_approved = matches!(level, ApprovalLevel::Safe | ApprovalLevel::NeedsApproval);
        assert!(auto_approved);
    }

    #[test]
    fn pattern_match_dangerous_requires_gate() {
        let level = ApprovalLevel::Dangerous;
        let needs_gate = matches!(level, ApprovalLevel::Dangerous);
        assert!(needs_gate);
    }

    #[tokio::test]
    async fn membrane_safe_passes_through() {
        let (tx, _rx) = mpsc::channel(8);
        let membrane = ApprovalMembrane::new(tx);
        let result = membrane
            .check(
                "c1".to_string(),
                "read_file",
                &serde_json::json!({}),
                ApprovalLevel::Safe,
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn membrane_needs_approval_passes_through() {
        let (tx, _rx) = mpsc::channel(8);
        let membrane = ApprovalMembrane::new(tx);
        let result = membrane
            .check(
                "c2".to_string(),
                "write_file",
                &serde_json::json!({}),
                ApprovalLevel::NeedsApproval,
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn membrane_dangerous_approved_returns_ok() {
        let (tx, mut rx) = mpsc::channel(8);
        let membrane = ApprovalMembrane::new(tx);

        // Spawn a task that auto-approves the first ApprovalRequested event.
        tokio::spawn(async move {
            if let Some(AgentEvent::ApprovalRequested { tx, .. }) = rx.recv().await {
                tx.send(true).expect("send approval");
            }
        });

        let result = membrane
            .check(
                "c3".to_string(),
                "delete_file",
                &serde_json::json!({"path": "/tmp/test"}),
                ApprovalLevel::Dangerous,
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn membrane_dangerous_denied_returns_error() {
        let (tx, mut rx) = mpsc::channel(8);
        let membrane = ApprovalMembrane::new(tx);

        tokio::spawn(async move {
            if let Some(AgentEvent::ApprovalRequested { tx, .. }) = rx.recv().await {
                tx.send(false).expect("send denial");
            }
        });

        let result = membrane
            .check(
                "c4".to_string(),
                "rm_rf",
                &serde_json::json!({"path": "/"}),
                ApprovalLevel::Dangerous,
            )
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("rm_rf"));
    }

    #[tokio::test]
    async fn membrane_dangerous_closed_channel_returns_error() {
        let (tx, rx) = mpsc::channel(8);
        // Drop rx so the channel is closed.
        drop(rx);
        let membrane = ApprovalMembrane::new(tx);

        let result = membrane
            .check(
                "c5".to_string(),
                "any_tool",
                &serde_json::json!({}),
                ApprovalLevel::Dangerous,
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn membrane_emits_approval_level_in_event() {
        let (tx, mut rx) = mpsc::channel(8);
        let membrane = ApprovalMembrane::new(tx);

        tokio::spawn(async move {
            if let Some(AgentEvent::ApprovalRequested {
                tx, approval_level, ..
            }) = rx.recv().await
            {
                assert_eq!(approval_level, ApprovalLevel::Dangerous);
                tx.send(true).expect("send");
            }
        });

        membrane
            .check(
                "c6".to_string(),
                "rm_rf",
                &serde_json::json!({}),
                ApprovalLevel::Dangerous,
            )
            .await
            .expect("approved");
    }

    #[tokio::test]
    async fn membrane_needs_approval_carries_level_in_event() {
        let (tx, mut rx) = mpsc::channel(8);
        let membrane = ApprovalMembrane::new(tx);

        tokio::spawn(async move {
            if let Some(AgentEvent::ApprovalRequested {
                tx, approval_level, ..
            }) = rx.recv().await
            {
                assert_eq!(approval_level, ApprovalLevel::NeedsApproval);
                tx.send(false).expect("send");
            }
        });

        let _ = membrane
            .check(
                "c7".to_string(),
                "write_file",
                &serde_json::json!({}),
                ApprovalLevel::NeedsApproval,
            )
            .await;
    }
}
