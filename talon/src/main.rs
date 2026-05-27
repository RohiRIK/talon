use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde_json::json;
use std::io::{self, Write as _};
use std::path::PathBuf;
use std::time::Duration;
use talon_core::approval::ApprovalLevel;
use talon_core::tools::ToolResult;
use talon_llm::{AnthropicProvider, ContentBlock, LlmProvider, Message};

// ── CLI definition ────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "talon",
    version,
    author,
    about = "Single-binary AI agent — persistent memory, multi-channel, WASM plugins.\nTRUST it. It's RUST.",
    long_about = None,
)]
struct Cli {
    /// Message to send to the agent (single-turn mode)
    #[arg(short, long)]
    message: Option<String>,

    /// Path to config file (default: ~/.talon/config.toml)
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Log level: error, warn, info, debug, trace
    #[arg(long, default_value = "info", value_name = "LEVEL")]
    log_level: String,

    /// Gateways to enable, comma-separated: cli,telegram,http,tui
    #[arg(long, default_value = "cli", value_name = "GATEWAYS")]
    gateway: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize Talon — creates ~/.talon/, writes starter config, stores API key in OS keychain.
    Init,
    /// Database maintenance utilities.
    Db {
        #[command(subcommand)]
        action: DbAction,
    },
    /// Memory utilities.
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
    /// Cache utilities.
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Diagnostics — checks API key, DB integrity, plugin health, network.
    Doctor,
}

#[derive(Subcommand)]
enum DbAction {
    /// Run VACUUM on the SQLite database to reclaim space.
    Vacuum,
    /// Print database statistics (size, row counts, FTS index).
    Stats,
}

#[derive(Subcommand)]
enum MemoryAction {
    /// Print memory statistics.
    Stats,
}

#[derive(Subcommand)]
enum CacheAction {
    /// Clear the semantic cache.
    Clear,
    /// Print cache statistics (hit rate, size).
    Stats,
}

// ── Concrete tool impls (enum dispatch; moves to talon-tools in Phase 3) ──────

enum BuiltinTool {
    Echo,
    ReadFile,
}

impl BuiltinTool {
    fn name(&self) -> &str {
        match self {
            Self::Echo => "echo",
            Self::ReadFile => "read_file",
        }
    }

    fn schema(&self) -> serde_json::Value {
        match self {
            Self::Echo => json!({
                "name": "echo",
                "description": "Echo back a message. Useful for verifying tool dispatch.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "message": { "type": "string", "description": "The message to echo." }
                    },
                    "required": ["message"]
                }
            }),
            Self::ReadFile => json!({
                "name": "read_file",
                "description": "Read the contents of a file from the local filesystem.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Path to the file to read." }
                    },
                    "required": ["path"]
                }
            }),
        }
    }

    fn approval_level(&self) -> ApprovalLevel {
        match self {
            Self::Echo | Self::ReadFile => ApprovalLevel::Safe,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> ToolResult {
        match self {
            Self::Echo => {
                let msg = args["message"].as_str().unwrap_or("(no message)");
                ToolResult::ok(msg)
            }
            Self::ReadFile => {
                let path = match args["path"].as_str() {
                    Some(p) if !p.is_empty() => p.to_string(),
                    _ => return ToolResult::err("Missing required argument: path"),
                };
                match std::fs::read_to_string(&path) {
                    Ok(content) => ToolResult::ok(content),
                    Err(e) => ToolResult::err(format!("Failed to read {path}: {e}")),
                }
            }
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    init_tracing(&cli.log_level)?;

    match cli.command {
        Some(Commands::Init) => cmd_init().await,
        Some(Commands::Db { action }) => cmd_db(action).await,
        Some(Commands::Memory { action }) => cmd_memory(action).await,
        Some(Commands::Cache { action }) => cmd_cache(action).await,
        Some(Commands::Doctor) => cmd_doctor().await,
        None => cmd_run(cli.message, cli.config, cli.gateway).await,
    }
}

// ── Subcommand implementations ────────────────────────────────────────────────

async fn cmd_init() -> Result<()> {
    let talon_dir = talon_home()?;
    if talon_dir.exists() {
        println!("~/.talon/ already exists — skipping directory creation.");
    } else {
        std::fs::create_dir_all(&talon_dir)
            .with_context(|| format!("failed to create {}", talon_dir.display()))?;
        println!("Created {}", talon_dir.display());
    }

    let config_path = talon_dir.join("config.toml");
    if !config_path.exists() {
        std::fs::write(&config_path, default_config())
            .with_context(|| format!("failed to write {}", config_path.display()))?;
        println!("Wrote starter config to {}", config_path.display());
    }

    let api_key = prompt_secret("Enter your Anthropic API key (sk-ant-...): ")?;
    if !api_key.is_empty() {
        store_api_key(&api_key).context("failed to store API key in OS keychain")?;
        println!("API key stored securely in OS keychain.");
    } else {
        println!("Skipped API key storage — set TALON_API_KEY env var to override.");
    }

    println!("\nTalon initialized. Run: talon --message \"hello\"");
    Ok(())
}

async fn cmd_db(action: DbAction) -> Result<()> {
    match action {
        DbAction::Vacuum => println!("talon db vacuum — not yet implemented (Phase 2)"),
        DbAction::Stats => println!("talon db stats — not yet implemented (Phase 2)"),
    }
    Ok(())
}

async fn cmd_memory(action: MemoryAction) -> Result<()> {
    match action {
        MemoryAction::Stats => println!("talon memory stats — not yet implemented (Phase 2.5)"),
    }
    Ok(())
}

async fn cmd_cache(action: CacheAction) -> Result<()> {
    match action {
        CacheAction::Clear => println!("talon cache clear — not yet implemented (Phase 2.5)"),
        CacheAction::Stats => println!("talon cache stats — not yet implemented (Phase 2.5)"),
    }
    Ok(())
}

async fn cmd_doctor() -> Result<()> {
    println!("talon doctor — not yet implemented (Phase 7)");
    Ok(())
}

async fn cmd_run(
    message: Option<String>,
    _config: Option<PathBuf>,
    _gateway: String,
) -> Result<()> {
    let Some(msg) = message else {
        println!("No message provided. Run `talon --help` for usage.");
        println!("Or run `talon init` to set up Talon.");
        return Ok(());
    };

    let api_key = std::env::var("TALON_LLM_API_KEY")
        .ok()
        .or_else(load_api_key)
        .unwrap_or_default();

    if api_key.is_empty() {
        println!("API key not configured. Run `talon init` or set TALON_LLM_API_KEY.");
        return Ok(());
    }

    run_agent(api_key, msg).await
}

// ── Agent loop ────────────────────────────────────────────────────────────────

fn check_approval(tool: &BuiltinTool, args: &serde_json::Value) -> Result<()> {
    match tool.approval_level() {
        ApprovalLevel::Safe => Ok(()),
        ApprovalLevel::NeedsApproval => {
            tracing::debug!(tool = tool.name(), %args, "auto-approving tool");
            Ok(())
        }
        ApprovalLevel::Dangerous => {
            eprint!("[talon] approve {}({args})? [y/n]: ", tool.name());
            io::stderr().flush().ok();
            let mut answer = String::new();
            io::stdin()
                .read_line(&mut answer)
                .context("failed to read approval input")?;
            if answer.trim().eq_ignore_ascii_case("y") {
                Ok(())
            } else {
                Err(anyhow::anyhow!("tool call denied by user"))
            }
        }
    }
}

async fn run_agent(api_key: String, user_message: String) -> Result<()> {
    let tools = [BuiltinTool::Echo, BuiltinTool::ReadFile];
    let tool_schemas: Vec<serde_json::Value> = tools.iter().map(|t| t.schema()).collect();
    let provider = AnthropicProvider::new(api_key);

    let mut messages: Vec<Message> = vec![Message::user(user_message)];

    loop {
        let response = tokio::time::timeout(
            Duration::from_secs(60),
            provider.complete(&messages, &tool_schemas),
        )
        .await
        .context("LLM request timed out")??;

        let assistant_content = serde_json::to_value(&response.content)
            .context("failed to serialize assistant content")?;
        messages.push(Message::assistant(assistant_content));

        if response.stop_reason == "end_turn" {
            for block in &response.content {
                if let ContentBlock::Text { text } = block {
                    println!("{text}");
                }
            }
            break;
        }

        let mut results: Vec<serde_json::Value> = Vec::new();
        for block in &response.content {
            let ContentBlock::ToolUse { id, name, input } = block else {
                continue;
            };
            let result = match tools.iter().find(|t| t.name() == name) {
                Some(t) => {
                    check_approval(t, &input)?;
                    t.execute(input.clone()).await
                }
                None => ToolResult::err(format!("Unknown tool: {name}")),
            };
            tracing::debug!(tool = %name, is_error = result.is_error, "tool executed");
            results.push(json!({
                "type": "tool_result",
                "tool_use_id": id,
                "content": result.content,
                "is_error": result.is_error,
            }));
        }

        if results.is_empty() {
            break;
        }
        messages.push(Message::user(serde_json::Value::Array(results)));
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn init_tracing(level: &str) -> Result<()> {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_new(level).or_else(|_| EnvFilter::try_new("info"))?;

    fmt().with_env_filter(filter).with_target(false).init();
    Ok(())
}

fn talon_home() -> Result<PathBuf> {
    let home = std::env::var("HOME").ok();
    let userprofile = std::env::var("USERPROFILE").ok();
    talon_home_from_env(home.as_deref(), userprofile.as_deref())
}

fn talon_home_from_env(home: Option<&str>, userprofile: Option<&str>) -> Result<PathBuf> {
    let base = home
        .or(userprofile)
        .context("could not determine home directory — set HOME or USERPROFILE")?;
    Ok(PathBuf::from(base).join(".talon"))
}

fn default_config() -> &'static str {
    r#"# Talon configuration — ~/.talon/config.toml
# Full reference: https://github.com/rohirikman/talon/blob/main/docs/config.md

[llm]
# Provider: anthropic | openai
provider = "anthropic"
# Model to use (leave empty for provider default)
model = ""
# Request timeout in seconds
timeout_secs = 60

[memory]
# SQLite database path (relative to ~/.talon/)
db_path = "talon.db"
# Maximum context window messages to include
context_messages = 20
# LTM (LanceDB) enabled — Phase 2.5
ltm_enabled = false

[gateway]
# HTTP gateway listen address
http_addr = "127.0.0.1:7777"
# Telegram — set token via TELEGRAM_BOT_TOKEN env var
telegram_enabled = false
"#
}

fn prompt_secret(prompt: &str) -> Result<String> {
    print!("{}", prompt);
    io::stdout().flush().context("failed to flush stdout")?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read input")?;
    Ok(input.trim().to_string())
}

fn store_api_key(key: &str) -> Result<()> {
    use keyring::Entry;
    let entry =
        Entry::new("talon", "anthropic-api-key").context("failed to create keyring entry")?;
    entry
        .set_password(key)
        .context("failed to store password")?;
    Ok(())
}

fn load_api_key() -> Option<String> {
    use keyring::Entry;
    Entry::new("talon", "anthropic-api-key")
        .ok()
        .and_then(|e| e.get_password().ok())
        .filter(|k| !k.is_empty())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── CLI parsing ───────────────────────────────────────────────────────────

    #[test]
    fn cli_parses_message_flag() -> Result<()> {
        let cli = Cli::try_parse_from(["talon", "--message", "hello world"])?;
        assert_eq!(cli.message.as_deref(), Some("hello world"));
        Ok(())
    }

    #[test]
    fn cli_message_is_none_when_absent() -> Result<()> {
        let cli = Cli::try_parse_from(["talon"])?;
        assert!(cli.message.is_none());
        Ok(())
    }

    #[test]
    fn cli_log_level_defaults_to_info() -> Result<()> {
        let cli = Cli::try_parse_from(["talon"])?;
        assert_eq!(cli.log_level, "info");
        Ok(())
    }

    #[test]
    fn cli_log_level_can_be_overridden() -> Result<()> {
        let cli = Cli::try_parse_from(["talon", "--log-level", "debug"])?;
        assert_eq!(cli.log_level, "debug");
        Ok(())
    }

    #[test]
    fn cli_gateway_defaults_to_cli() -> Result<()> {
        let cli = Cli::try_parse_from(["talon"])?;
        assert_eq!(cli.gateway, "cli");
        Ok(())
    }

    #[test]
    fn cli_gateway_accepts_multiple_values() -> Result<()> {
        let cli = Cli::try_parse_from(["talon", "--gateway", "cli,telegram"])?;
        assert_eq!(cli.gateway, "cli,telegram");
        Ok(())
    }

    #[test]
    fn cli_parses_init_subcommand() -> Result<()> {
        let cli = Cli::try_parse_from(["talon", "init"])?;
        assert!(matches!(cli.command, Some(Commands::Init)));
        Ok(())
    }

    #[test]
    fn cli_parses_db_vacuum() -> Result<()> {
        let cli = Cli::try_parse_from(["talon", "db", "vacuum"])?;
        assert!(matches!(
            cli.command,
            Some(Commands::Db {
                action: DbAction::Vacuum
            })
        ));
        Ok(())
    }

    #[test]
    fn cli_parses_db_stats() -> Result<()> {
        let cli = Cli::try_parse_from(["talon", "db", "stats"])?;
        assert!(matches!(
            cli.command,
            Some(Commands::Db {
                action: DbAction::Stats
            })
        ));
        Ok(())
    }

    #[test]
    fn cli_parses_memory_stats() -> Result<()> {
        let cli = Cli::try_parse_from(["talon", "memory", "stats"])?;
        assert!(matches!(
            cli.command,
            Some(Commands::Memory {
                action: MemoryAction::Stats
            })
        ));
        Ok(())
    }

    #[test]
    fn cli_parses_cache_clear() -> Result<()> {
        let cli = Cli::try_parse_from(["talon", "cache", "clear"])?;
        assert!(matches!(
            cli.command,
            Some(Commands::Cache {
                action: CacheAction::Clear
            })
        ));
        Ok(())
    }

    #[test]
    fn cli_parses_cache_stats() -> Result<()> {
        let cli = Cli::try_parse_from(["talon", "cache", "stats"])?;
        assert!(matches!(
            cli.command,
            Some(Commands::Cache {
                action: CacheAction::Stats
            })
        ));
        Ok(())
    }

    #[test]
    fn cli_parses_doctor() -> Result<()> {
        let cli = Cli::try_parse_from(["talon", "doctor"])?;
        assert!(matches!(cli.command, Some(Commands::Doctor)));
        Ok(())
    }

    #[test]
    fn cli_rejects_unknown_flag() {
        let result = Cli::try_parse_from(["talon", "--does-not-exist"]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_config_accepts_path() -> Result<()> {
        let cli = Cli::try_parse_from(["talon", "--config", "/tmp/talon.toml"])?;
        assert_eq!(cli.config, Some(PathBuf::from("/tmp/talon.toml")));
        Ok(())
    }

    // ── talon_home ────────────────────────────────────────────────────────────

    #[test]
    fn talon_home_ends_with_dottalon() -> Result<()> {
        let path = talon_home()?;
        assert_eq!(
            path.file_name().and_then(|s| s.to_str()),
            Some(".talon"),
            "talon_home() must end in .talon, got {path:?}"
        );
        Ok(())
    }

    #[test]
    fn talon_home_is_absolute() -> Result<()> {
        let path = talon_home()?;
        assert!(
            path.is_absolute(),
            "talon_home() must be an absolute path, got {path:?}"
        );
        Ok(())
    }

    #[test]
    fn talon_home_uses_userprofile_when_home_absent() -> Result<()> {
        let path = talon_home_from_env(None, Some("/tmp/testuser"))?;
        assert_eq!(path, PathBuf::from("/tmp/testuser/.talon"));
        Ok(())
    }

    #[test]
    fn talon_home_prefers_home_over_userprofile() -> Result<()> {
        let path = talon_home_from_env(Some("/tmp/primary"), Some("/tmp/secondary"))?;
        assert_eq!(path, PathBuf::from("/tmp/primary/.talon"));
        Ok(())
    }

    #[test]
    fn talon_home_errors_when_both_absent() {
        let result = talon_home_from_env(None, None);
        assert!(result.is_err());
    }

    // ── default_config ────────────────────────────────────────────────────────

    #[test]
    fn default_config_is_nonempty() {
        assert!(!default_config().is_empty());
    }

    #[test]
    fn default_config_has_llm_section() {
        assert!(
            default_config().contains("[llm]"),
            "default_config must contain [llm] section"
        );
    }

    #[test]
    fn default_config_has_memory_section() {
        assert!(
            default_config().contains("[memory]"),
            "default_config must contain [memory] section"
        );
    }

    #[test]
    fn default_config_has_gateway_section() {
        assert!(
            default_config().contains("[gateway]"),
            "default_config must contain [gateway] section"
        );
    }

    #[test]
    fn default_config_provider_is_anthropic() {
        assert!(default_config().contains("provider = \"anthropic\""));
    }

    // ── stub command handlers ─────────────────────────────────────────────────

    #[tokio::test]
    async fn cmd_db_vacuum_returns_ok() -> Result<()> {
        cmd_db(DbAction::Vacuum).await
    }

    #[tokio::test]
    async fn cmd_db_stats_returns_ok() -> Result<()> {
        cmd_db(DbAction::Stats).await
    }

    #[tokio::test]
    async fn cmd_memory_stats_returns_ok() -> Result<()> {
        cmd_memory(MemoryAction::Stats).await
    }

    #[tokio::test]
    async fn cmd_cache_clear_returns_ok() -> Result<()> {
        cmd_cache(CacheAction::Clear).await
    }

    #[tokio::test]
    async fn cmd_cache_stats_returns_ok() -> Result<()> {
        cmd_cache(CacheAction::Stats).await
    }

    #[tokio::test]
    async fn cmd_doctor_returns_ok() -> Result<()> {
        cmd_doctor().await
    }

    #[tokio::test]
    async fn cmd_run_with_no_message_returns_ok() -> Result<()> {
        cmd_run(None, None, "cli".to_string()).await
    }

    #[tokio::test]
    async fn cmd_run_with_message_returns_ok_without_api_key() -> Result<()> {
        // Full integration test: TALON_LLM_API_KEY=sk-... cargo run -- --message "hello"
        if std::env::var("TALON_LLM_API_KEY").is_ok() {
            return Ok(());
        }
        cmd_run(Some("hello".to_string()), None, "cli".to_string()).await
    }

    // ── BuiltinTool ───────────────────────────────────────────────────────────

    #[test]
    fn echo_tool_name() {
        assert_eq!(BuiltinTool::Echo.name(), "echo");
    }

    #[test]
    fn read_file_tool_name() {
        assert_eq!(BuiltinTool::ReadFile.name(), "read_file");
    }

    #[test]
    fn echo_schema_has_required_message() {
        let s = BuiltinTool::Echo.schema();
        let required = &s["input_schema"]["required"];
        assert!(
            required
                .as_array()
                .map(|a| a.iter().any(|v| v == "message"))
                .unwrap_or(false)
        );
    }

    #[test]
    fn read_file_schema_has_required_path() {
        let s = BuiltinTool::ReadFile.schema();
        let required = &s["input_schema"]["required"];
        assert!(
            required
                .as_array()
                .map(|a| a.iter().any(|v| v == "path"))
                .unwrap_or(false)
        );
    }

    #[tokio::test]
    async fn echo_tool_returns_message() {
        let result = BuiltinTool::Echo.execute(json!({ "message": "hi" })).await;
        assert!(!result.is_error);
        assert_eq!(result.content, "hi");
    }

    #[tokio::test]
    async fn echo_tool_handles_missing_message() {
        let result = BuiltinTool::Echo.execute(json!({})).await;
        assert!(!result.is_error);
        assert_eq!(result.content, "(no message)");
    }

    #[tokio::test]
    async fn read_file_tool_reads_existing_file() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "hello from file")?;
        let path_str = path
            .to_str()
            .context("temp path is not valid UTF-8")?
            .to_string();
        let result = BuiltinTool::ReadFile
            .execute(json!({ "path": path_str }))
            .await;
        assert!(!result.is_error);
        assert_eq!(result.content, "hello from file");
        Ok(())
    }

    #[tokio::test]
    async fn read_file_tool_errors_on_missing_file() {
        let result = BuiltinTool::ReadFile
            .execute(json!({ "path": "/nonexistent/path/xyz.txt" }))
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("Failed to read"));
    }

    #[tokio::test]
    async fn read_file_tool_errors_on_empty_path() {
        let result = BuiltinTool::ReadFile.execute(json!({ "path": "" })).await;
        assert!(result.is_error);
        assert!(result.content.contains("Missing required argument"));
    }

    #[tokio::test]
    async fn read_file_tool_errors_on_missing_path_arg() {
        let result = BuiltinTool::ReadFile.execute(json!({})).await;
        assert!(result.is_error);
        assert!(result.content.contains("Missing required argument"));
    }

    #[test]
    fn check_approval_safe_always_ok() {
        assert!(check_approval(&BuiltinTool::Echo, &json!({"message": "hi"})).is_ok());
        assert!(check_approval(&BuiltinTool::ReadFile, &json!({"path": "/tmp/x"})).is_ok());
    }

    #[test]
    fn all_prototype_tools_are_safe() {
        assert!(matches!(
            BuiltinTool::Echo.approval_level(),
            ApprovalLevel::Safe
        ));
        assert!(matches!(
            BuiltinTool::ReadFile.approval_level(),
            ApprovalLevel::Safe
        ));
    }
}
