//! Minimal MCP JSON-RPC 2.0 client over stdio or HTTP.

use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

/// MCP protocol version Talon speaks.
const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Debug, Error)]
pub enum McpError {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("server error {code}: {message}")]
    Server { code: i64, message: String },
}

/// A tool definition as reported by an MCP server (`tools/list`).
#[derive(Debug, Clone, Deserialize)]
pub struct McpToolDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Value,
}

/// How to reach an MCP server.
pub enum McpTransport {
    Http { url: String },
    Stdio { command: String, args: Vec<String> },
}

struct StdioConn {
    // Kept alive for the connection's lifetime; killed on drop.
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
}

enum Conn {
    Http {
        client: reqwest::Client,
        url: String,
    },
    Stdio(Box<Mutex<StdioConn>>),
}

/// A connected MCP client. `Send + Sync` so it can live in an `Arc` shared by
/// many `McpToolAdapter`s.
pub struct McpClient {
    conn: Conn,
    next_id: AtomicU64,
}

impl McpClient {
    /// Connect over the given transport. For stdio this spawns the server
    /// process; for HTTP it just records the endpoint.
    pub async fn connect(transport: McpTransport) -> Result<Self, McpError> {
        let conn = match transport {
            McpTransport::Http { url } => Conn::Http {
                client: reqwest::Client::new(),
                url,
            },
            McpTransport::Stdio { command, args } => {
                let mut child = Command::new(&command)
                    .args(&args)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| McpError::Transport(format!("spawn '{command}': {e}")))?;
                let stdin = child
                    .stdin
                    .take()
                    .ok_or_else(|| McpError::Transport("child has no stdin".into()))?;
                let stdout = child
                    .stdout
                    .take()
                    .ok_or_else(|| McpError::Transport("child has no stdout".into()))?;
                Conn::Stdio(Box::new(Mutex::new(StdioConn {
                    _child: child,
                    stdin,
                    stdout: BufReader::new(stdout).lines(),
                })))
            }
        };
        Ok(Self {
            conn,
            next_id: AtomicU64::new(1),
        })
    }

    /// Send one JSON-RPC request and return its `result` (or an error).
    async fn request(&self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });

        let resp: Value = match &self.conn {
            Conn::Http { client, url } => {
                let r = client
                    .post(url)
                    .json(&req)
                    .send()
                    .await
                    .map_err(|e| McpError::Transport(e.to_string()))?;
                if !r.status().is_success() {
                    return Err(McpError::Transport(format!("HTTP {}", r.status().as_u16())));
                }
                r.json()
                    .await
                    .map_err(|e| McpError::Protocol(format!("invalid JSON: {e}")))?
            }
            Conn::Stdio(m) => {
                let mut conn = m.lock().await;
                let mut line =
                    serde_json::to_string(&req).map_err(|e| McpError::Protocol(e.to_string()))?;
                line.push('\n');
                conn.stdin
                    .write_all(line.as_bytes())
                    .await
                    .map_err(|e| McpError::Transport(e.to_string()))?;
                conn.stdin
                    .flush()
                    .await
                    .map_err(|e| McpError::Transport(e.to_string()))?;

                // Read until the response with our id arrives (skip notifications/logs).
                loop {
                    let next = conn
                        .stdout
                        .next_line()
                        .await
                        .map_err(|e| McpError::Transport(e.to_string()))?;
                    let Some(l) = next else {
                        return Err(McpError::Transport("server closed stdout".into()));
                    };
                    if l.trim().is_empty() {
                        continue;
                    }
                    let Ok(v) = serde_json::from_str::<Value>(&l) else {
                        continue;
                    };
                    if v.get("id").and_then(|x| x.as_u64()) == Some(id) {
                        break v;
                    }
                }
            }
        };

        if let Some(err) = resp.get("error") {
            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error")
                .to_string();
            return Err(McpError::Server { code, message });
        }
        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Perform the MCP `initialize` handshake.
    pub async fn initialize(&self) -> Result<Value, McpError> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "talon", "version": env!("CARGO_PKG_VERSION") }
        });
        self.request("initialize", params).await
    }

    /// List the server's tools.
    pub async fn list_tools(&self) -> Result<Vec<McpToolDef>, McpError> {
        let result = self.request("tools/list", json!({})).await?;
        let tools = result.get("tools").cloned().unwrap_or(Value::Array(vec![]));
        serde_json::from_value(tools)
            .map_err(|e| McpError::Protocol(format!("bad tools/list: {e}")))
    }

    /// Call a tool and return its concatenated text content.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<String, McpError> {
        let result = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await?;
        match result.get("content").and_then(|c| c.as_array()) {
            Some(parts) => Ok(parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")),
            None => Ok(result.to_string()),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn http_initialize_list_and_call() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "initialize" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0", "id": 1, "result": { "protocolVersion": "2024-11-05" }
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "tools/list" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0", "id": 2, "result": { "tools": [
                    { "name": "echo", "description": "Echo a string", "inputSchema": { "type": "object" } }
                ]}
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({ "method": "tools/call" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0", "id": 3, "result": { "content": [
                    { "type": "text", "text": "hello" }
                ]}
            })))
            .mount(&server)
            .await;

        let client = McpClient::connect(McpTransport::Http { url: server.uri() })
            .await
            .unwrap();
        client.initialize().await.unwrap();

        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
        assert_eq!(tools[0].input_schema["type"], "object");

        let out = client
            .call_tool("echo", json!({ "msg": "hi" }))
            .await
            .unwrap();
        assert_eq!(out, "hello");
    }

    #[tokio::test]
    async fn http_server_error_propagates() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0", "id": 1, "error": { "code": -32601, "message": "Method not found" }
            })))
            .mount(&server)
            .await;

        let client = McpClient::connect(McpTransport::Http { url: server.uri() })
            .await
            .unwrap();
        let err = client.list_tools().await.unwrap_err();
        assert!(
            matches!(err, McpError::Server { code: -32601, .. }),
            "got: {err}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stdio_initialize_list_and_call() {
        // A tiny MCP server in sh: echo back the request id, branch on method.
        let script = r#"while IFS= read -r line; do
            id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
            case "$line" in
              *tools/list*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"E","inputSchema":{"type":"object"}}]}}\n' "$id" ;;
              *tools/call*) printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"stdio-hello"}]}}\n' "$id" ;;
              *) printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id" ;;
            esac
          done"#;

        let client = McpClient::connect(McpTransport::Stdio {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), script.to_string()],
        })
        .await
        .unwrap();

        client.initialize().await.unwrap();
        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools[0].name, "echo");
        let out = client.call_tool("echo", json!({})).await.unwrap();
        assert_eq!(out, "stdio-hello");
    }
}
