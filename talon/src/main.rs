use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde_json::json;
use std::future::Future;
use std::io::{self, Write as _};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use talon_core::approval::ApprovalLevel;
use talon_core::tools::{Tool, ToolContext, ToolResult};
use talon_gateway::{Gateway, GatewayContext, cli::CliGateway, http::HttpGateway, tui::TuiGateway};
use talon_llm::{AnthropicProvider, GitHubCopilotProvider, LlmProvider};
use talon_memory::{Database, SqliteStore};
use talon_tools::SessionSearchTool;

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

    /// Gateway to use: cli, tui, http
    #[arg(long, default_value = "cli", value_name = "GATEWAY")]
    gateway: String,

    /// Accessible mode — line-by-line output, no TUI escape sequences
    #[arg(long)]
    accessible: bool,

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

// ── Concrete tool impls (moves to talon-tools in Phase 3) ────────────────────

struct EchoTool;

impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn schema(&self) -> serde_json::Value {
        json!({
            "name": "echo",
            "description": "Echo back a message. Useful for verifying tool dispatch.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "message": { "type": "string", "description": "The message to echo." }
                },
                "required": ["message"]
            }
        })
    }
    fn approval_level(&self, _args: &serde_json::Value) -> ApprovalLevel {
        ApprovalLevel::Safe
    }
    fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        Box::pin(async move {
            let msg = args["message"].as_str().unwrap_or("(no message)");
            ToolResult::ok(msg)
        })
    }
}

struct ReadFileTool;

impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn schema(&self) -> serde_json::Value {
        json!({
            "name": "read_file",
            "description": "Read the contents of a file from the local filesystem.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to read." }
                },
                "required": ["path"]
            }
        })
    }
    fn approval_level(&self, _args: &serde_json::Value) -> ApprovalLevel {
        ApprovalLevel::Safe
    }
    fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        Box::pin(async move {
            let path = match args["path"].as_str() {
                Some(p) if !p.is_empty() => p.to_string(),
                _ => return ToolResult::err("Missing required argument: path"),
            };
            match std::fs::read_to_string(&path) {
                Ok(content) => ToolResult::ok(content),
                Err(e) => ToolResult::err(format!("Failed to read {path}: {e}")),
            }
        })
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
        None => cmd_run(cli.message, cli.config, cli.gateway, cli.accessible).await,
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
    let db_path = default_db_path();
    let db = Database::open(db_path.to_str().unwrap_or(":memory:"))
        .map_err(|e| anyhow::anyhow!("failed to open database: {e}"))?;
    db.init_schema()
        .await
        .map_err(|e| anyhow::anyhow!("failed to run migrations: {e}"))?;
    cmd_db_with(&db, action).await
}

async fn cmd_db_with(db: &Database, action: DbAction) -> Result<()> {
    match action {
        DbAction::Vacuum => {
            db.vacuum()
                .await
                .map_err(|e| anyhow::anyhow!("vacuum failed: {e}"))?;
            println!("vacuum complete");
        }
        DbAction::Stats => {
            let s = db
                .stats()
                .await
                .map_err(|e| anyhow::anyhow!("stats failed: {e}"))?;
            println!("sessions : {}", s.session_count);
            println!("messages : {}", s.message_count);
            println!("size     : {} KB", s.size_bytes / 1024);
        }
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
    gateway_flag: String,
    accessible: bool,
) -> Result<()> {
    let provider_name =
        std::env::var("TALON_LLM_PROVIDER").unwrap_or_else(|_| "anthropic".to_string());

    // Key-less providers (github-copilot, claude-code) don't need TALON_LLM_API_KEY.
    let needs_api_key = matches!(provider_name.as_str(), "anthropic" | "openai");

    let api_key = if needs_api_key {
        let key = std::env::var("TALON_LLM_API_KEY")
            .ok()
            .or_else(load_api_key)
            .unwrap_or_default();
        if key.is_empty() {
            println!(
                "API key not configured. Run `talon init`, set TALON_LLM_API_KEY, \
                 or use a key-less provider: TALON_LLM_PROVIDER=github-copilot"
            );
            return Ok(());
        }
        key
    } else {
        String::new()
    };

    let ctx = build_gateway_context(&provider_name, api_key).await?;
    let ctx = Arc::new(ctx);

    // Single-turn mode: --message "..." skips the interactive REPL.
    if let Some(msg) = message {
        let cli = CliGateway::new(Arc::clone(&ctx), talon_gateway::RenderMode::Plain);
        cli.run_turn_pub(msg)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        return Ok(());
    }

    // Multi-turn mode: choose gateway based on --gateway flag.
    let gateway: Arc<dyn Gateway> = match gateway_flag.as_str() {
        "http" => {
            let addr = "127.0.0.1:7777".parse().context("invalid HTTP addr")?;
            Arc::new(HttpGateway::new(Arc::clone(&ctx), addr))
        }
        "tui" => Arc::new(TuiGateway::new(Arc::clone(&ctx), accessible, "talon")),
        #[cfg(feature = "telegram")]
        "telegram" => {
            let gw = talon_gateway::telegram::TelegramGateway::from_env(Arc::clone(&ctx))
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            Arc::new(gw)
        }
        _ => {
            // "cli" and any unknown value fall back to CLI.
            let mode = if accessible {
                talon_gateway::RenderMode::Accessible
            } else {
                talon_gateway::RenderMode::Plain
            };
            Arc::new(CliGateway::new(Arc::clone(&ctx), mode))
        }
    };

    gateway.run().await.map_err(|e| anyhow::anyhow!("{e}"))
}

async fn build_gateway_context(provider_name: &str, api_key: String) -> Result<GatewayContext> {
    let provider: Arc<dyn LlmProvider> = match provider_name {
        "github-copilot" | "copilot" => Arc::new(
            GitHubCopilotProvider::new()
                .map_err(|e| anyhow::anyhow!("GitHub Copilot auth failed: {e}"))?,
        ),
        // "anthropic" and anything else defaults to Anthropic.
        _ => Arc::new(AnthropicProvider::new(api_key)),
    };
    let mut ctx = GatewayContext::new(provider);

    // Register built-in tools.
    ctx = ctx.with_tool(Arc::new(EchoTool));
    ctx = ctx.with_tool(Arc::new(ReadFileTool));

    // Set up DB persistence and register memory tools.
    let db = talon_home()
        .ok()
        .map(|p| p.join("talon.db"))
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .and_then(|path| Database::open(&path).ok())
        .map(Arc::new);

    if let Some(ref db) = db {
        db.init_schema().await.ok();
        let store = Arc::new(SqliteStore::new(Arc::clone(db)));
        ctx = ctx.with_tool(Arc::new(SessionSearchTool::new(store)));
        ctx = ctx.with_db(Arc::clone(db));
    }

    Ok(ctx)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn init_tracing(level: &str) -> Result<()> {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_new(level).or_else(|_| EnvFilter::try_new("info"))?;

    fmt().with_env_filter(filter).with_target(false).init();
    Ok(())
}

fn default_db_path() -> PathBuf {
    talon_home()
        .map(|p| p.join("talon.db"))
        .unwrap_or_else(|_| PathBuf::from(":memory:"))
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
    fn cli_gateway_accepts_http_value() -> Result<()> {
        let cli = Cli::try_parse_from(["talon", "--gateway", "http"])?;
        assert_eq!(cli.gateway, "http");
        Ok(())
    }

    #[test]
    fn cli_accessible_flag_defaults_false() -> Result<()> {
        let cli = Cli::try_parse_from(["talon"])?;
        assert!(!cli.accessible);
        Ok(())
    }

    #[test]
    fn cli_accessible_flag_set() -> Result<()> {
        let cli = Cli::try_parse_from(["talon", "--accessible"])?;
        assert!(cli.accessible);
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
        let db = Database::open(":memory:").map_err(|e| anyhow::anyhow!("{e}"))?;
        db.init_schema().await.map_err(|e| anyhow::anyhow!("{e}"))?;
        cmd_db_with(&db, DbAction::Vacuum).await
    }

    #[tokio::test]
    async fn cmd_db_stats_returns_ok() -> Result<()> {
        let db = Database::open(":memory:").map_err(|e| anyhow::anyhow!("{e}"))?;
        db.init_schema().await.map_err(|e| anyhow::anyhow!("{e}"))?;
        cmd_db_with(&db, DbAction::Stats).await
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
        cmd_run(None, None, "cli".to_string(), false).await
    }

    #[tokio::test]
    async fn cmd_run_with_message_returns_ok_without_api_key() -> Result<()> {
        // Full integration test: TALON_LLM_API_KEY=sk-... cargo run -- --message "hello"
        if std::env::var("TALON_LLM_API_KEY").is_ok() {
            return Ok(());
        }
        cmd_run(Some("hello".to_string()), None, "cli".to_string(), false).await
    }

    // ── Tool impls ────────────────────────────────────────────────────────────

    #[test]
    fn echo_tool_name() {
        assert_eq!(EchoTool.name(), "echo");
    }

    #[test]
    fn read_file_tool_name() {
        assert_eq!(ReadFileTool.name(), "read_file");
    }

    #[test]
    fn echo_schema_has_required_message() {
        let s = EchoTool.schema();
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
        let s = ReadFileTool.schema();
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
        let result = EchoTool
            .execute(json!({ "message": "hi" }), ToolContext::default())
            .await;
        assert!(!result.is_error);
        assert_eq!(result.content, "hi");
    }

    #[tokio::test]
    async fn echo_tool_handles_missing_message() {
        let result = EchoTool.execute(json!({}), ToolContext::default()).await;
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
        let result = ReadFileTool
            .execute(json!({ "path": path_str }), ToolContext::default())
            .await;
        assert!(!result.is_error);
        assert_eq!(result.content, "hello from file");
        Ok(())
    }

    #[tokio::test]
    async fn read_file_tool_errors_on_missing_file() {
        let result = ReadFileTool
            .execute(
                json!({ "path": "/nonexistent/path/xyz.txt" }),
                ToolContext::default(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("Failed to read"));
    }

    #[tokio::test]
    async fn read_file_tool_errors_on_empty_path() {
        let result = ReadFileTool
            .execute(json!({ "path": "" }), ToolContext::default())
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("Missing required argument"));
    }

    #[tokio::test]
    async fn read_file_tool_errors_on_missing_path_arg() {
        let result = ReadFileTool
            .execute(json!({}), ToolContext::default())
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("Missing required argument"));
    }

    #[test]
    fn all_builtin_tools_are_safe() {
        let args = json!({});
        assert_eq!(EchoTool.approval_level(&args), ApprovalLevel::Safe);
        assert_eq!(ReadFileTool.approval_level(&args), ApprovalLevel::Safe);
    }
}
