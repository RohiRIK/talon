# MCP Protocol Integration

> **Status:** ✅ Complete
> **Category:** API Bindings

---

## 1. What is MCP?

Model Context Protocol (MCP) is an open standard by Anthropic for connecting
AI models to external tools and data sources. MCP servers expose:
- **Tools** (callable functions)
- **Resources** (readable data)
- **Prompts** (reusable templates)

Talon acts as an **MCP client** — it connects to MCP servers and exposes
their tools to the LLM alongside Talon's built-in tools.

---

## 2. Transport Types

MCP supports two transports:

| Transport | Use Case |
|-----------|---------|
| `stdio` | Local server process (subprocess) |
| `http+sse` | Remote server over HTTP |

---

## 3. MCP Client Implementation

```rust
// talon-core/src/mcp/client.rs

pub struct McpClient {
    transport: McpTransport,
    server_name: String,
}

pub enum McpTransport {
    Stdio {
        process: tokio::process::Child,
        stdin: tokio::process::ChildStdin,
        stdout: BufReader<tokio::process::ChildStdout>,
    },
    Http {
        base_url: String,
        http: reqwest::Client,
    },
}

impl McpClient {
    /// Launch a stdio MCP server
    pub async fn spawn_stdio(
        server_name: &str,
        command: &str,
        args: &[&str],
        env: HashMap<String, String>,
    ) -> Result<Self, McpError> {
        let mut child = tokio::process::Command::new(command)
            .args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;

        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());

        let mut client = Self {
            server_name: server_name.to_string(),
            transport: McpTransport::Stdio { process: child, stdin, stdout },
        };

        // MCP initialization handshake
        client.initialize().await?;
        Ok(client)
    }

    async fn initialize(&mut self) -> Result<(), McpError> {
        // Send initialize request
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "clientInfo": { "name": "talon", "version": env!("CARGO_PKG_VERSION") }
            }
        });
        self.send_request(req).await?;

        // Send initialized notification
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        self.send_notification(notif).await
    }

    pub async fn list_tools(&mut self) -> Result<Vec<McpToolSpec>, McpError> {
        let resp = self.send_request(json!({
            "jsonrpc": "2.0",
            "id": self.next_id(),
            "method": "tools/list"
        })).await?;

        let tools = resp["result"]["tools"].as_array()
            .ok_or(McpError::InvalidResponse)?;

        Ok(tools.iter().filter_map(|t| {
            serde_json::from_value(t.clone()).ok()
        }).collect())
    }

    pub async fn call_tool(
        &mut self,
        name: &str,
        args: Value,
    ) -> Result<ToolResult, McpError> {
        let resp = self.send_request(json!({
            "jsonrpc": "2.0",
            "id": self.next_id(),
            "method": "tools/call",
            "params": { "name": name, "arguments": args }
        })).await?;

        let content = &resp["result"]["content"][0];
        let text = content["text"].as_str().unwrap_or("").to_string();
        let is_error = resp["result"]["isError"].as_bool().unwrap_or(false);

        Ok(if is_error {
            ToolResult::error(text)
        } else {
            ToolResult::success(text)
        })
    }
}
```

---

## 4. MCP Tool Adapter

Wraps an MCP tool as an Talon `Tool`:

```rust
pub struct McpToolAdapter {
    client: Arc<Mutex<McpClient>>,
    spec: McpToolSpec,
}

#[async_trait]
impl Tool for McpToolAdapter {
    fn name(&self) -> &str { &self.spec.name }
    fn description(&self) -> &str { &self.spec.description }
    fn schema(&self) -> Value { self.spec.input_schema.clone() }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult {
        let mut client = self.client.lock().await;
        client.call_tool(&self.spec.name, args).await
            .unwrap_or_else(|e| ToolResult::error(e.to_string()))
    }
}
```

---

## 5. Configuration

```toml
# Connect to local filesystem MCP server
[[mcp.servers]]
name = "filesystem"
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/rohi/projects"]

# Connect to GitHub MCP server
[[mcp.servers]]
name = "github"
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_PERSONAL_ACCESS_TOKEN = "${env:GITHUB_TOKEN}" }

# Connect to remote HTTP MCP server
[[mcp.servers]]
name = "remote-tools"
transport = "http"
base_url = "https://mcp.example.com"
```
---

## Related Documents

### See Also
- [MCP Client Tool](../04_Core_Features/36_MCP_Client_Tool.md)
- [ACP Protocol Integration](48_ACP_Protocol_Integration.md)
- [Tool System Architecture](../02_Architecture/16_Tool_System_Architecture.md)

