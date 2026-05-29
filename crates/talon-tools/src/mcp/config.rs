//! `~/.talon/mcp_servers.toml` — declares MCP servers to connect to.
//!
//! ```toml
//! [[server]]
//! name = "filesystem"
//! transport = "stdio"
//! command = "npx"
//! args = ["-y", "@modelcontextprotocol/server-filesystem", "/path"]
//!
//! [[server]]
//! name = "remote"
//! transport = "http"
//! url = "https://example.com/mcp"
//! ```

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::client::{McpError, McpTransport};

/// Parsed `mcp_servers.toml`.
#[derive(Debug, Default, Deserialize)]
pub struct McpServersConfig {
    #[serde(default)]
    pub server: Vec<McpServerEntry>,
}

/// One `[[server]]` entry.
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerEntry {
    pub name: String,
    pub transport: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
    /// Optional approval override (informational; the adapter defaults to
    /// `NeedsApproval`).
    #[serde(default)]
    pub approval: Option<String>,
}

impl McpServerEntry {
    /// Build the transport this entry describes.
    pub fn to_transport(&self) -> Result<McpTransport, McpError> {
        match self.transport.as_str() {
            "stdio" => {
                let command = self.command.clone().ok_or_else(|| {
                    McpError::Protocol(format!(
                        "server '{}': stdio transport requires 'command'",
                        self.name
                    ))
                })?;
                Ok(McpTransport::Stdio {
                    command,
                    args: self.args.clone(),
                })
            }
            "http" => {
                let url = self.url.clone().ok_or_else(|| {
                    McpError::Protocol(format!(
                        "server '{}': http transport requires 'url'",
                        self.name
                    ))
                })?;
                Ok(McpTransport::Http { url })
            }
            other => Err(McpError::Protocol(format!(
                "server '{}': unknown transport '{other}'",
                self.name
            ))),
        }
    }
}

impl McpServersConfig {
    /// Parse from a TOML string.
    pub fn parse(toml_str: &str) -> Result<Self, McpError> {
        toml::from_str(toml_str)
            .map_err(|e| McpError::Protocol(format!("invalid mcp_servers.toml: {e}")))
    }

    /// Load from a file. A missing file is not an error — it yields an empty
    /// config (no servers configured).
    pub fn load(path: &Path) -> Result<Self, McpError> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::parse(&s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(McpError::Transport(format!(
                "reading {}: {e}",
                path.display()
            ))),
        }
    }

    /// Default location: `~/.talon/mcp_servers.toml`.
    pub fn default_path() -> PathBuf {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_default();
        home.join(".talon").join("mcp_servers.toml")
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        [[server]]
        name = "fs"
        transport = "stdio"
        command = "npx"
        args = ["-y", "server-fs", "/tmp"]

        [[server]]
        name = "remote"
        transport = "http"
        url = "https://example.com/mcp"
        approval = "needs_approval"
    "#;

    #[test]
    fn parses_stdio_and_http_entries() {
        let cfg = McpServersConfig::parse(SAMPLE).unwrap();
        assert_eq!(cfg.server.len(), 2);
        assert_eq!(cfg.server[0].name, "fs");
        assert!(matches!(
            cfg.server[0].to_transport().unwrap(),
            McpTransport::Stdio { .. }
        ));
        assert!(matches!(
            cfg.server[1].to_transport().unwrap(),
            McpTransport::Http { .. }
        ));
    }

    #[test]
    fn stdio_without_command_is_error() {
        let toml = r#"
            [[server]]
            name = "broken"
            transport = "stdio"
        "#;
        let cfg = McpServersConfig::parse(toml).unwrap();
        assert!(cfg.server[0].to_transport().is_err());
    }

    #[test]
    fn unknown_transport_is_error() {
        let toml = r#"
            [[server]]
            name = "x"
            transport = "carrier-pigeon"
        "#;
        let cfg = McpServersConfig::parse(toml).unwrap();
        assert!(cfg.server[0].to_transport().is_err());
    }

    #[test]
    fn malformed_toml_is_error() {
        assert!(McpServersConfig::parse("this = = not toml").is_err());
    }

    #[test]
    fn missing_file_yields_empty_config() {
        let cfg = McpServersConfig::load(Path::new("/no/such/talon/mcp_servers.toml")).unwrap();
        assert!(cfg.server.is_empty());
    }
}
