//! MCP (Model Context Protocol) client and tool adapter (Phase 5).
//!
//! `client` speaks minimal JSON-RPC 2.0 over stdio or HTTP. `adapter` (task
//! 5.4) wraps a server's tools as `Arc<dyn Tool>`, and `config` (task 5.6)
//! parses `~/.talon/mcp_servers.toml`.

pub mod adapter;
pub mod client;
pub mod config;

pub use adapter::{McpToolAdapter, adapt_server};
pub use client::{McpClient, McpError, McpToolDef, McpTransport};
pub use config::{McpServerEntry, McpServersConfig};
