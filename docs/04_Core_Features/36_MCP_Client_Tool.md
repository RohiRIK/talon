# MCP Client — Model Context Protocol Integration

> **Status:** ✅ Complete
> **Category:** Core Features
> **Last corrected:** dogfood pass 3

---

## 1. Overview

Talon implements an MCP *client* — it connects to external MCP *servers*
and exposes their tools natively inside the agent's tool registry.

From the LLM's perspective, an MCP-provided tool is indistinguishable
from a native Talon tool. The adapter layer handles protocol translation.

```
Talon Agent Loop
      │
      ▼
ToolRegistry
      ├── NativeTool (terminal, read_file, ...)
      └── McpToolAdapter
              │
              ▼
        McpServerConnection
              │
    ┌─────────┴──────────┐
    │ stdio transport     │  (local process)
    │ HTTP/SSE transport  │  (remote server)
    └────────────────────┘
```

---

## 2. MCP Server Configuration

```toml
# config.toml

[[mcp.servers]]
name = "filesystem"
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/rohi/projects"]
env = {}

[[mcp.servers]]
name = "github"
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "${GITHUB_TOKEN}" }

[[mcp.servers]]
name = "postgres"
transport = "http"
url = "http://localhost:5173"
headers = { Authorization = "Bearer ${POSTGRES_MCP_TOKEN}" }
```

---

## 3. McpServerConnection

```rust
use mcp_client::{McpClient, StdioTransport, HttpTransport};

pub enum McpTransport {
    Stdio(StdioTransport),
    Http(HttpTransport),
}

pub struct McpServerConnection {
    name: String,
    client: Arc<McpClient>,
    tools: Vec<McpToolDef>,
}

impl McpServerConnection {
    pub async fn connect(config: &McpServerConfig) -> Result<Self, McpError> {
        let client = match &config.transport {
            TransportConfig::Stdio { command, args, env } => {
                let transport = StdioTransport::new(command, args, env).await?;
                McpClient::new(transport)
            }
            TransportConfig::Http { url, headers } => {
                let transport = HttpTransport::new(url, headers.clone());
                McpClient::new(transport)
            }
        };

        // Initialize handshake
        client.initialize(mcp_client::InitializeParams {
            client_info: mcp_client::ClientInfo {
                name: "talon".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            capabilities: Default::default(),
        }).await?;

        // Discover tools
        let tools_resp = client.list_tools().await?;

        Ok(Self {
            name: config.name.clone(),
            client: Arc::new(client),
            tools: tools_resp.tools,
        })
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tools.iter()
            .map(|t| format!("{}:{}", self.name, t.name))
            .collect()
    }
}
```

---

## 4. McpToolAdapter — implements Tool trait

```rust
pub struct McpToolAdapter {
    server_name: String,
    tool_def: McpToolDef,
    client: Arc<McpClient>,
}

#[async_trait]
impl Tool for McpToolAdapter {
    fn name(&self) -> &str {
        // Namespaced: "github:create_issue"
        // (stored as field to avoid lifetime issues)
        &self.qualified_name
    }

    fn description(&self) -> &str {
        &self.tool_def.description
    }

    fn schema(&self) -> serde_json::Value {
        // MCP tools carry their own JSON schema — use directly
        self.tool_def.input_schema.clone()
    }

    fn approval_level(&self) -> ApprovalLevel {
        // MCP tools are conservatively rated Confirmation (LocalWrite-class)
        // unless overridden in config
        ApprovalLevel::Confirmation
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let result = self.client
            .call_tool(mcp_client::CallToolParams {
                name: self.tool_def.name.clone(),
                arguments: Some(args),
            })
            .await
            .map_err(|e| ToolError::McpError(e.to_string()))?;

        // Convert MCP content blocks to ToolResult
        let text = result.content.iter().filter_map(|block| {
            match block {
                mcp_client::ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            }
        }).collect::<Vec<_>>().join("\n");

        Ok(ToolResult::text(text))
    }
}
```

---

## 5. Dynamic Tool Registration

```rust
pub struct McpManager {
    connections: RwLock<HashMap<String, McpServerConnection>>,
}

impl McpManager {
    pub async fn load_all(
        &self,
        configs: &[McpServerConfig],
        registry: &mut ToolRegistry,
    ) -> Vec<McpLoadError> {
        let mut errors = vec![];

        for config in configs {
            match McpServerConnection::connect(config).await {
                Ok(conn) => {
                    tracing::info!(
                        server = config.name,
                        tools = conn.tools.len(),
                        "MCP server connected"
                    );

                    // Register each MCP tool as a native tool
                    for tool_def in &conn.tools {
                        let adapter = McpToolAdapter::new(
                            config.name.clone(),
                            tool_def.clone(),
                            conn.client.clone(),
                        );
                        registry.register(Arc::new(adapter));
                    }

                    self.connections.write().await
                        .insert(config.name.clone(), conn);
                }
                Err(e) => {
                    tracing::warn!(server = config.name, error = %e, "MCP server failed");
                    errors.push(McpLoadError { server: config.name.clone(), error: e });
                }
            }
        }

        errors
    }

    /// Hot-reload: reconnect a server and re-register its tools
    pub async fn reload_server(
        &self,
        name: &str,
        registry: &mut ToolRegistry,
    ) -> Result<(), McpError> {
        // Remove old tools
        registry.remove_prefix(&format!("{name}:"));

        // Reconnect
        let config = self.get_config(name)?;
        let conn = McpServerConnection::connect(&config).await?;

        for tool_def in &conn.tools {
            let adapter = McpToolAdapter::new(name.to_string(), tool_def.clone(), conn.client.clone());
            registry.register(Arc::new(adapter));
        }

        Ok(())
    }
}
```

---

## 6. native_mcp Tool (user-facing)

Talon also exposes a meta-tool for managing MCP servers at runtime:

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NativeMcpParams {
    pub action: McpAction,
    pub server: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpAction {
    /// List all connected servers and their tools
    List,
    /// Reconnect a server
    Reload,
    /// Disconnect a server
    Disconnect,
}
```

---

## 7. Crate Dependency

```toml
# talon-tools/Cargo.toml
[dependencies]
mcp-client = { version = "0.1", features = ["stdio", "http"] }
# or build your own minimal client:
# tokio-process for stdio, reqwest for HTTP/SSE
```

> **Last corrected:** dogfood pass 4

> **Note:** The MCP Rust client ecosystem is early (2025–2026).
> If `mcp-client` is immature, fall back to a thin hand-rolled client
> that implements only `initialize`, `tools/list`, and `tools/call`.
> The protocol is simple JSON-RPC 2.0 over stdio or HTTP+SSE.
---

## Related Documents

### Depends On
- [Tool System Architecture](../02_Architecture/16_Tool_System_Architecture.md)

### See Also
- [MCP Protocol Integration](../05_API_Bindings/47_MCP_Protocol_Integration.md)
- [ACP Protocol Integration](../05_API_Bindings/48_ACP_Protocol_Integration.md)

