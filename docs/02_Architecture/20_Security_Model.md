# Security Model & Trust Boundaries

> **Last corrected:** dogfood pass 4

> **Status:** ✅ Complete
> **Category:** Architecture

---

## 1. Trust Zones

```
Zone 0 — Talon Core (TRUSTED)
  │  Owns: config, memory, approval logic
  │  Cannot be influenced by LLM output directly
  │
  ├─ Zone 1 — Tool Inputs (SEMI-TRUSTED)
  │    LLM-provided tool arguments.
  │    Validated against JSON Schema before execution.
  │    String arguments sanitized for shell injection.
  │
  ├─ Zone 2 — Tool Outputs (UNTRUSTED)
  │    Subprocess stdout/stderr returned to LLM context.
  │    Truncated to prevent context stuffing.
  │    Never executed as code by Talon itself.
  │
  └─ Zone 3 — External Platforms (UNTRUSTED)
       Messages from Telegram, Discord, webhooks.
       Rate-limited per sender.
       Privilege escalation blocked (see §3).
```

---

## 2. The Approval Membrane (Full Spec)

```rust
pub struct ApprovalMembrane {
    pub default_level: ApprovalLevel,
    pub tool_overrides: HashMap<String, ApprovalLevel>,
    pub session_approvals: HashSet<String>,  // "approved for this session"
    pub pending_tx: mpsc::Sender<ApprovalRequest>,
}

#[derive(Debug, Clone)]
pub enum ApprovalLevel {
    Safe,
    NeedsApproval,
    Dangerous,
}

pub enum ToolRisk {
    ReadOnly,       // web_search, read_file, skill_view — always allowed
    LocalWrite,     // write_file, memory — allowed by default
    Destructive,    // terminal, browser, send_message — ask before executing
    Irreversible,   // git push, deploy, delete — always ask
}

impl ApprovalMembrane {
    pub async fn check(&self, call: &ToolCall, risk: ToolRisk) -> ApprovalDecision {
        // 1. Check tool-specific override
        if let Some(level) = self.tool_overrides.get(&call.name) {
            return self.decide(call, level, risk).await;
        }

        // 2. Use default level
        self.decide(call, &self.default_level, risk).await
    }

    async fn decide(
        &self, call: &ToolCall, level: &ApprovalLevel, risk: ToolRisk,
    ) -> ApprovalDecision {
        match (level, risk) {
            (ApprovalLevel::Safe, _) => ApprovalDecision::Approved,
            (_, ToolRisk::ReadOnly) => ApprovalDecision::Approved,
            (ApprovalLevel::NeedsApproval, _) => {
                if self.session_approvals.contains(&call.name) {
                    ApprovalDecision::Approved
                } else {
                    ApprovalDecision::NeedUserApproval(format!(
                        "Allow `{}` for this session?", call.name
                    ))
                }
            }
            (ApprovalLevel::Dangerous, risk) if risk >= ToolRisk::Destructive => {
                ApprovalDecision::NeedUserApproval(format!(
                    "Allow `{}` with args:\n```json\n{}\n```",
                    call.name,
                    serde_json::to_string_pretty(&call.arguments).unwrap_or_default()
                ))
            }
            _ => ApprovalDecision::Approved,
        }
    }
}
```

---

## 3. Privilege Escalation Prevention

An LLM cannot instruct Talon to:
- Change its own approval level
- Disable safety checks
- Access config/secrets files
- Execute code outside the sandbox

Enforced by:
```rust
// In TerminalTool::execute()
fn is_prohibited(cmd: &str) -> bool {
    let patterns = [
        "talon config",      // no self-reconfiguration
        "rm -rf /",           // obvious
        "chmod 777",          // permission escalation
        "curl * | sh",        // remote code execution
        "eval ",              // dynamic code execution
        "sudo",               // privilege escalation (configurable)
    ];
    patterns.iter().any(|p| cmd.contains(p))
}
```

Additionally, the `Irreversible` risk tier always requires explicit approval regardless of `--yolo` (unless the user explicitly configures `force_approve_irreversible = true` in config).

---

## 4. Docker Sandbox Backend

For `terminal` tool execution in high-security contexts:

```rust
pub struct DockerSandboxBackend {
    docker: Docker,  // bollard client
    image: String,   // e.g. "talon-sandbox:latest"
    network: Option<String>,  // None = no network access
    memory_limit: u64,        // bytes
    cpu_quota: i64,           // microseconds per 100ms
}

impl TerminalBackend for DockerSandboxBackend {
    async fn execute(&self, cmd: &str, workdir: Option<&str>) -> Result<ExecResult, ToolError> {
        let container = self.docker.create_container(
            None::<CreateContainerOptions<String>>,
            Config {
                image: Some(self.image.clone()),
                cmd: Some(vec!["sh".into(), "-c".into(), cmd.into()]),
                working_dir: workdir.map(|s| s.into()),
                host_config: Some(HostConfig {
                    memory: Some(self.memory_limit as i64),
                    cpu_quota: Some(self.cpu_quota),
                    network_mode: self.network.clone().or(Some("none".into())),
                    readonly_rootfs: Some(true),
                    security_opt: Some(vec!["no-new-privileges".into()]),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ).await?;

        // Start, wait, collect output, remove container
        self.docker.start_container(&container.id, None::<StartContainerOptions<String>>).await?;
        let status = self.docker.wait_container(&container.id, None::<WaitContainerOptions<String>>)
            .next().await.transpose()?;
        // ... collect stdout/stderr ...
        self.docker.remove_container(&container.id, None).await?;
        Ok(result)
    }
}
```

---

## 5. Secret Management

Secrets (API keys) never enter the LLM context:

```rust
pub struct SecretStore {
    inner: HashMap<String, String>,
}

impl SecretStore {
    pub fn from_env() -> Self {
        // Load from .env file + environment variables
        // Strips secrets from any value before it reaches tool args
    }

    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_string();
        for secret in self.inner.values() {
            if secret.len() > 8 {
                out = out.replace(secret.as_str(), "[REDACTED]");
            }
        }
        out
    }
}
```

`SecretStore::redact()` is called on all tool output before it is returned to the LLM context. No API key can accidentally leak into a conversation log.

---

## 6. Rate Limiting Per Sender

```rust
pub struct RateLimiter {
    // Per sender: sliding window counter
    windows: Arc<Mutex<HashMap<String, SlidingWindow>>>,
    limits: RateLimitConfig,
}

pub struct RateLimitConfig {
    pub messages_per_minute: u32,
    pub messages_per_hour: u32,
    pub max_concurrent_sessions: u32,
}

impl RateLimiter {
    pub fn check(&self, sender_id: &str) -> Result<(), GatewayError> {
        let mut windows = self.windows.lock().unwrap();
        let window = windows.entry(sender_id.to_string())
            .or_insert_with(SlidingWindow::new);

        if window.count_last_minute() >= self.limits.messages_per_minute {
            return Err(GatewayError::RateLimited {
                retry_after: window.seconds_until_reset(),
            });
        }
        window.record();
        Ok(())
    }
}
```
---

## Related Documents

### Depends On
- [Plugin & Skill Architecture](17_Plugin_And_Skill_Architecture.md)
- [Tool System Architecture](16_Tool_System_Architecture.md)

### See Also
- [Terminal Tool (Docker Sandbox)](../04_Core_Features/30a_Terminal_Tool.md)
- [Docker & Container Deployment](../08_DevOps/61_Docker_And_Container_Deployment.md)
- [Approval Membrane](17a_Approval_Membrane.md)

