//! `McpToolAdapter` — expose an MCP server's tools as Talon `Arc<dyn Tool>`.

use std::sync::Arc;
use std::{future::Future, pin::Pin};

use serde_json::{Value, json};
use talon_core::{
    approval::ApprovalLevel,
    tools::{Tool, ToolContext, ToolResult},
};

use super::client::{McpClient, McpError, McpToolDef};

/// Wraps a single MCP tool. `execute` forwards to the shared client's
/// `tools/call`. `NeedsApproval` — remote tools are never auto-approved.
pub struct McpToolAdapter {
    client: Arc<McpClient>,
    def: McpToolDef,
}

impl McpToolAdapter {
    pub fn new(client: Arc<McpClient>, def: McpToolDef) -> Self {
        Self { client, def }
    }
}

impl Tool for McpToolAdapter {
    fn name(&self) -> &str {
        &self.def.name
    }

    fn schema(&self) -> Value {
        json!({
            "name": self.def.name,
            "description": self.def.description,
            "input_schema": self.def.input_schema,
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
        let client = Arc::clone(&self.client);
        let name = self.def.name.clone();
        Box::pin(async move {
            match client.call_tool(&name, args).await {
                Ok(content) => ToolResult::ok(content),
                Err(e) => ToolResult::err(format!("mcp tool '{name}' failed: {e}")),
            }
        })
    }
}

/// Initialize a server and adapt every tool it advertises into `Arc<dyn Tool>`.
pub async fn adapt_server(client: Arc<McpClient>) -> Result<Vec<Arc<dyn Tool>>, McpError> {
    client.initialize().await?;
    let defs = client.list_tools().await?;
    Ok(defs
        .into_iter()
        .map(|def| Arc::new(McpToolAdapter::new(Arc::clone(&client), def)) as Arc<dyn Tool>)
        .collect())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::mcp::client::McpTransport;
    use wiremock::matchers::{body_partial_json, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn mock_server(call_response: Value) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "initialize" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0", "id": 1, "result": {}
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "tools/list" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0", "id": 2, "result": { "tools": [
                    { "name": "weather", "description": "Get weather", "inputSchema": { "type": "object" } }
                ]}
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "tools/call" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(call_response))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn adapts_tools_and_executes() {
        let server = mock_server(json!({
            "jsonrpc": "2.0", "id": 3, "result": { "content": [ { "type": "text", "text": "sunny" } ] }
        }))
        .await;

        let client = Arc::new(
            McpClient::connect(McpTransport::Http { url: server.uri() })
                .await
                .unwrap(),
        );
        let tools = adapt_server(client).await.unwrap();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "weather");
        assert_eq!(tools[0].schema()["name"], "weather");
        assert_eq!(
            tools[0].approval_level(&Value::Null),
            ApprovalLevel::NeedsApproval
        );

        let r = tools[0]
            .execute(json!({ "city": "x" }), ToolContext::default())
            .await;
        assert!(!r.is_error, "got: {}", r.content);
        assert_eq!(r.content, "sunny");
    }

    #[tokio::test]
    async fn tool_call_error_maps_to_err() {
        let server = mock_server(json!({
            "jsonrpc": "2.0", "id": 3, "error": { "code": -32000, "message": "boom" }
        }))
        .await;

        let client = Arc::new(
            McpClient::connect(McpTransport::Http { url: server.uri() })
                .await
                .unwrap(),
        );
        let tools = adapt_server(client).await.unwrap();
        let r = tools[0].execute(json!({}), ToolContext::default()).await;
        assert!(r.is_error);
        assert!(r.content.contains("boom"), "got: {}", r.content);
    }
}
