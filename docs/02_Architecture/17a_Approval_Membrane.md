# Approval Membrane — Safety & Trust Layer

> **Last corrected:** dogfood pass 4

> **Status:** ✅ Complete
> **Category:** Architecture

---

## 1. Purpose

The Approval Membrane is Talon's safety layer between the LLM's intent
and actual system effects. Every tool call passes through it before execution.

Design goals:
- **Zero trust by default** — no tool is pre-approved unless explicitly configured
- **Risk classification** — tools carry static risk annotations
- **User agency** — users can approve/deny individual calls or set blanket policies
- **Async-safe** — approval requests go through channels, not blocking stdin

---

## 2. Risk Taxonomy

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolRisk {
    /// Read-only, no external effects (read_file, search_files, web_search)
    ReadOnly = 0,
    /// Writes to local state (write_file, patch, memory)
    LocalWrite = 1,
    /// Network I/O or can cause irreversible damage (terminal, send_message, delete, browser_navigate)
    Destructive = 2,
    /// Permanently irreversible (git push --force, deploy, rm -rf)
    Irreversible = 3,
}
```

---

## 3. Approval Level Configuration

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalLevel {
    Safe,
    NeedsApproval,
    Dangerous,
}

impl Default for ApprovalLevel {
    fn default() -> Self { Self::Confirmation }
}
```

Per-tool overrides allow fine-grained control:

```toml
[approval]
default_level = "confirmation"

# Always approve these regardless of risk
always_approve = ["read_file", "search_files", "web_search", "memory"]  # maps to Safe level

# Always ask for these even if level is Safe
always_ask = ["terminal", "send_message"]

# Never allow — hard block
never_allow = ["rm_rf_tool"]
```

---

## 4. ApprovalMembrane Implementation

```rust
pub struct ApprovalMembrane {
    level: ApprovalLevel,
    always_approve: HashSet<String>,
    always_ask: HashSet<String>,
    never_allow: HashSet<String>,
    /// Channel for sending approval requests to the UI
    request_tx: mpsc::Sender<ApprovalRequest>,
    /// Pending approvals awaiting user response
    pending: DashMap<Uuid, oneshot::Sender<ApprovalDecision>>,
}

pub enum ApprovalDecision {
    Approved,
    ApprovedForSession,  // remember this tool as approved for this session
    Denied,
    DeniedWithReason(String),
}

pub struct ApprovalRequest {
    pub id: Uuid,
    pub tool_name: String,
    pub risk: ToolRisk,
    pub arguments: serde_json::Value,
    pub response_tx: oneshot::Sender<ApprovalDecision>,
}
```

---

## 5. Decision Logic

```rust
impl ApprovalMembrane {
    pub async fn check(
        &self,
        tool_name: &str,
        risk: ToolRisk,
        args: &serde_json::Value,
    ) -> Result<(), ApprovalError> {
        // Hard block
        if self.never_allow.contains(tool_name) {
            return Err(ApprovalError::HardBlocked(tool_name.to_string()));
        }

        // Check session-level approvals first (user said "yes for this session")
        if self.session_approved.contains(tool_name) {
            return Ok(());
        }

        // Hard approve
        if self.always_approve.contains(tool_name) {
            return Ok(());
        }

        // Hard ask
        let must_ask = self.always_ask.contains(tool_name);

        let decision = match self.level {
            ApprovalLevel::Safe if !must_ask => {
                return Ok(());
            }
            ApprovalLevel::NeedsApproval if !must_ask => {
                if risk < ToolRisk::Destructive {
                    return Ok(()); // auto-approve safe tools
                }
                self.ask_user(tool_name, risk, args).await?
            }
            ApprovalLevel::Dangerous => {
                return Err(ApprovalError::HardBlocked(tool_name.to_string()));
            }
            _ => {
                self.ask_user(tool_name, risk, args).await?
            }
        };

        match decision {
            ApprovalDecision::Approved => Ok(()),
            ApprovalDecision::ApprovedForSession => {
                self.session_approved.insert(tool_name.to_string());
                Ok(())
            }
            ApprovalDecision::Denied => Err(ApprovalError::Denied),
            ApprovalDecision::DeniedWithReason(r) => Err(ApprovalError::DeniedWithReason(r)),
        }
    }

    async fn ask_user(
        &self,
        tool_name: &str,
        risk: ToolRisk,
        args: &serde_json::Value,
    ) -> Result<ApprovalDecision, ApprovalError> {
        let id = Uuid::new_v4();
        let (response_tx, response_rx) = oneshot::channel();

        self.pending.insert(id, response_tx);

        self.request_tx.send(ApprovalRequest {
            id,
            tool_name: tool_name.to_string(),
            risk,
            arguments: args.clone(),
            response_tx: /* ... */,
        }).await.map_err(|_| ApprovalError::UiDisconnected)?;

        // Wait for user response with timeout
        tokio::time::timeout(
            Duration::from_secs(300), // 5 minute timeout
            response_rx,
        )
        .await
        .map_err(|_| ApprovalError::Timeout)?
        .map_err(|_| ApprovalError::UiDisconnected)
    }
}
```

---

## 6. UI Rendering of Approval Requests

The CLI gateway renders approval requests inline:

```
┌─────────────────────────────────────────────────────────┐
│  ⚠️  APPROVAL REQUIRED                                   │
│                                                          │
│  Tool:   terminal (Destructive)                          │
│  Command: git push origin main --force                   │
│                                                          │
│  [A] Approve   [S] Approve for session                   │
│  [D] Deny      [R] Deny with reason                      │
└─────────────────────────────────────────────────────────┘
```

Telegram gateway sends inline keyboard buttons:

```rust
async fn send_approval_prompt(
    &self,
    chat_id: ChatId,
    req: &ApprovalRequest,
) -> Result<(), GatewayError> {
    let preview = format_args_preview(&req.arguments, 200);
    let text = format!(
        "⚠️ *Approval Required*\n\nTool: `{}`\nRisk: {:?}\n\n```\n{}\n```",
        req.tool_name, req.risk, preview
    );

    let keyboard = InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("✅ Approve", format!("approve:{}", req.id)),
        InlineKeyboardButton::callback("🔒 Session", format!("approve_session:{}", req.id)),
        InlineKeyboardButton::callback("❌ Deny", format!("deny:{}", req.id)),
    ]]);

    self.bot
        .send_message(chat_id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}
```

---

## 7. Headless / Cron Mode

When Talon runs a [cron job](../04_Core_Features/33_Cron_Scheduler.md), there is no user present to approve.
The membrane auto-configures to `Safe` level for cron sessions,
but the session's `ApprovalLevel` can be overridden per-job:

```rust
impl CronJobBuilder {
    pub fn with_approval_level(mut self, level: ApprovalLevel) -> Self {
        self.approval_level = level;
        self
    }
}

// Cron sessions default to Safe (auto-approve)
let membrane = ApprovalMembrane::headless();
let ctx = ToolContext {
    membrane: Arc::new(membrane),
    session_type: SessionType::Cron,
    ..
};
```

---

## 8. Audit Log

Every approval decision is persisted to SQLite:

```sql
CREATE TABLE approval_log (
    id          TEXT PRIMARY KEY,
    session_id  TEXT NOT NULL,
    tool_name   TEXT NOT NULL,
    risk_level  TEXT NOT NULL,
    arguments   TEXT NOT NULL,
    decision    TEXT NOT NULL,
    decided_at  INTEGER,
    latency_ms  INTEGER,
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);
```

This feeds into the [self-evolution](../04_Core_Features/39_Self_Evolution_Loop.md) trajectory collection system —
patterns of approvals and denials become training signals.

---

## 9. Integration with Agent Loop

```rust
// Inside agent_loop.rs — tool execution path
async fn execute_tool_call(
    &self,
    call: &ToolCall,
    ctx: &ToolContext,
) -> Result<ToolResult, AgentError> {
    let tool = self.tools.get(&call.name)
        .ok_or_else(|| AgentError::UnknownTool(call.name.clone()))?;

    // 1. Approval check — may block waiting for user
    ctx.membrane.check(&call.name, tool.risk_level(), &call.arguments)
        .await
        .map_err(|e| AgentError::ApprovalDenied {
            tool: call.name.clone(),
            reason: e.to_string(),
        })?;

    // 2. Execute
    let output = tool.execute(call.arguments.clone(), ctx)
        .await
        .map_err(AgentError::ToolFailed)?;

    // 3. Record to trajectory
    ctx.trajectory.record_tool_call(call, &output).await;

    Ok(output)
}
```
---

## Related Documents

### Depends On
- [Tool System Architecture](16_Tool_System_Architecture.md)
- [Security Model](20_Security_Model.md)

### See Also
- [Plugin & Skill Architecture](17_Plugin_And_Skill_Architecture.md)
- [Terminal Tool](../04_Core_Features/30a_Terminal_Tool.md)
- [Gateway Architecture](18_Gateway_MultiChannel_Architecture.md)

