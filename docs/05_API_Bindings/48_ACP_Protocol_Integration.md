# ACP Protocol Integration

> **Status:** ✅ Complete
> **Category:** API Bindings

---

## 1. What is ACP?

ACP (Agent Communication Protocol) is a protocol for agent-to-agent
communication. Unlike MCP (which connects models to tools), ACP connects
agents to other agents — enabling Talon to delegate tasks to specialized
sub-agents implemented in any language or framework.

Talon implements ACP in two modes:
1. **Client** — Talon spawns an ACP-compatible subprocess and delegates tasks
2. **Server** — Talon exposes an ACP interface so other orchestrators can delegate to it

---

## 2. ACP Client (Talon delegates to other agents)

```rust
pub struct AcpClient {
    process: tokio::process::Child,
    stdin: tokio::io::BufWriter<tokio::process::ChildStdin>,
    stdout: tokio::io::BufReader<tokio::process::ChildStdout>,
    pending: HashMap<u64, oneshot::Sender<AcpResponse>>,
    next_id: u64,
}

impl AcpClient {
    pub async fn spawn(command: &str, args: &[&str]) -> Result<Self, AcpError> {
        let mut process = tokio::process::Command::new(command)
            .args(args)
            .arg("--acp")
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;

        Ok(Self {
            stdin: BufWriter::new(process.stdin.take().unwrap()),
            stdout: BufReader::new(process.stdout.take().unwrap()),
            process,
            pending: HashMap::new(),
            next_id: 1,
        })
    }

    pub async fn run_task(&mut self, goal: &str) -> Result<String, AcpError> {
        let id = self.next_id;
        self.next_id += 1;

        let request = json!({
            "id": id,
            "method": "run",
            "params": { "goal": goal }
        });

        // Write JSON-RPC request + newline
        let line = serde_json::to_string(&request)? + "\n";
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;

        // Read response
        let mut response_line = String::new();
        self.stdout.read_line(&mut response_line).await?;
        let response: AcpResponse = serde_json::from_str(&response_line)?;

        Ok(response.result.output)
    }
}
```

---

## 3. ACP Server (Talon accepts delegation)

```rust
// Run Talon in ACP server mode: talon --acp --stdio
pub async fn run_acp_server(agent: Arc<AgentLoop>) -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();
    let mut writer = BufWriter::new(stdout);

    while let Some(line) = reader.next_line().await? {
        let request: AcpRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let error = json!({
                    "id": null,
                    "error": { "code": -32700, "message": format!("Parse error: {e}") }
                });
                writer.write_all((error.to_string() + "\n").as_bytes()).await?;
                writer.flush().await?;
                continue;
            }
        };

        match request.method.as_str() {
            "run" => {
                let goal = request.params["goal"].as_str().unwrap_or("").to_string();
                let result = agent.run_simple(goal).await
                    .unwrap_or_else(|e| e.to_string());

                let response = json!({
                    "id": request.id,
                    "result": { "output": result }
                });
                writer.write_all((response.to_string() + "\n").as_bytes()).await?;
                writer.flush().await?;
            }
            other => {
                let error = json!({
                    "id": request.id,
                    "error": { "code": -32601, "message": format!("Unknown method: {other}") }
                });
                writer.write_all((error.to_string() + "\n").as_bytes()).await?;
                writer.flush().await?;
            }
        }
    }

    Ok(())
}
```

---

## 4. The delegate_task Tool

The `[delegate_task](../04_Core_Features/37_Subagent_Delegation.md)` tool uses ACP to spawn specialized sub-agents:

```rust
pub struct DelegateTaskTool {
    acp_command: Option<String>,  // e.g., "copilot" for GitHub Copilot CLI
    max_spawn_depth: u32,
}

#[async_trait]
impl Tool for DelegateTaskTool {
    fn name(&self) -> &str { "delegate_task" }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let params: DelegateTaskParams = serde_json::from_value(args)?;

        if ctx.spawn_depth >= self.max_spawn_depth {
            return ToolResult::error(format!(
                "Max spawn depth {} reached. Nested delegation disabled.",
                self.max_spawn_depth
            ));
        }

        // Launch sub-agent via ACP
        let cmd = self.acp_command.as_deref().unwrap_or("talon");
        let mut client = AcpClient::spawn(cmd, &["--acp", "--stdio"]).await?;
        let result = client.run_task(&params.goal).await?;

        ToolResult::success(result)
    }
}
```

---

## 5. Configuration

```toml
[tools.delegate_task]
enabled = true
max_spawn_depth = 1         # nested sub-agents disabled by default
max_concurrent = 3

# Override ACP command (e.g., use GitHub Copilot as sub-agent)
acp_command = "copilot"     # or "talon" (default, self-delegation)
acp_args = ["--acp", "--stdio"]
```
---

## Related Documents

### See Also
- [MCP Protocol Integration](47_MCP_Protocol_Integration.md)
- [Subagent Delegation](../04_Core_Features/37_Subagent_Delegation.md)

