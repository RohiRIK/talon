use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde_json::json;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use talon_core::approval::ApprovalLevel;
use talon_core::events::AgentEvent;
use talon_core::scheduler::{JobOutcome, JobRunner, RunEvent, Scheduler};
use talon_core::tools::{Tool, ToolContext, ToolResult};
use talon_gateway::web::{ApprovalBroker, EVENT_CHANNEL_CAP, PendingApproval, WebState};
use talon_gateway::{Gateway, GatewayContext, cli::CliGateway, http::HttpGateway, tui::TuiGateway};
use talon_llm::{
    AnthropicProvider, FallbackProvider, GitHubCopilotProvider, LlmConfig, LlmProvider,
    OpenAiCompatProvider, ProviderChoice,
};
use talon_memory::{CronJob, CronStore, Database, LtmStore, RunStore, SqliteStore};
use talon_tools::mcp::{McpClient, McpServersConfig, adapt_server};
use talon_tools::web::WebConfig;
use talon_tools::{CronJobTool, SessionSearchTool, WebExtractTool, WebSearchTool, timeouts};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

mod cron_cli;
mod logging;
mod secret_cli;
mod wizard;

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
    /// Run the long-lived daemon: cron scheduler + a foreground gateway.
    Serve {
        /// Gateway to run alongside the scheduler: cli, tui, http, telegram.
        #[arg(long, default_value = "http", value_name = "GATEWAY")]
        gateway: String,
    },
    /// Inspect and manage scheduled jobs (rendered as a tree).
    Cron {
        #[command(subcommand)]
        action: cron_cli::CronAction,
    },
    /// Manage the builtin encrypted secret vault.
    Secret {
        #[command(subcommand)]
        action: secret_cli::SecretAction,
    },
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
        Some(Commands::Serve { gateway }) => cmd_serve(gateway, cli.accessible).await,
        Some(Commands::Cron { action }) => cron_cli::run(action).await,
        Some(Commands::Secret { action }) => secret_cli::run(action, talon_home()?).await,
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

    // Web console token: generate once, preserve forever (fail-closed API
    // means no token = no /api/v1, so first-run must not leave it blank).
    match ensure_api_token(&config_path) {
        Ok(true) => println!(
            "Generated [gateway] api_token — the web console (localhost-only by default) \
             requires it as a Bearer token."
        ),
        Ok(false) => {}
        Err(e) => tracing::warn!("could not ensure [gateway] api_token: {e}"),
    }

    // Vault master key — credential first, then keygen (criterion 1).
    // Failure here must not abort provider setup.
    if let Err(e) = secret_cli::init_vault_bootstrap(&talon_dir) {
        tracing::warn!("vault bootstrap failed: {e}");
        println!("Vault setup failed ({e}) — rerun `talon init` after fixing the issue.");
    }

    match wizard::run_provider_wizard().await {
        Ok(Some(cfg)) => {
            let existing = std::fs::read_to_string(&config_path)
                .unwrap_or_else(|_| default_config().to_string());
            let merged = wizard::merge_llm_into_config(&existing, &cfg)?;
            std::fs::write(&config_path, merged)
                .with_context(|| format!("failed to write {}", config_path.display()))?;
            println!("Saved provider chain to {}", config_path.display());
        }
        Ok(None) => {
            println!("No providers selected — rerun `talon init` or set TALON_LLM_PROVIDER.");
        }
        Err(e) => {
            // Non-interactive terminal (CI, piped stdin) or cancellation: fall
            // back to the env-var path rather than failing init.
            tracing::warn!("provider wizard skipped: {e}");
            println!("Skipped interactive setup — set TALON_LLM_PROVIDER + TALON_LLM_API_KEY.");
        }
    }

    println!("\nTalon initialized. Run: talon --message \"hello\"");
    Ok(())
}

/// Insert a generated `[gateway] api_token` into the config if absent.
/// Returns `true` when a new token was written; an existing token is never
/// touched. The token is random (UUIDv4, no dashes) — not derived from
/// anything on the machine.
fn ensure_api_token(config_path: &std::path::Path) -> Result<bool> {
    let existing = std::fs::read_to_string(config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;
    let mut doc: toml::Table = toml::from_str(&existing).context("config.toml is not valid TOML")?;

    let gateway = doc
        .entry("gateway")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let table = gateway
        .as_table_mut()
        .context("[gateway] is not a table in config.toml")?;
    let has_token = table
        .get("api_token")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    if has_token {
        return Ok(false);
    }

    let token = format!("talon_{}", uuid::Uuid::new_v4().simple());
    table.insert("api_token".to_string(), toml::Value::String(token));
    let merged = toml::to_string_pretty(&doc).context("serialize merged config")?;
    std::fs::write(config_path, merged)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(true)
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
    let db_path = default_db_path();
    let db = Database::open(db_path.to_str().unwrap_or(":memory:"))
        .map_err(|e| anyhow::anyhow!("failed to open database: {e}"))?;
    db.init_schema()
        .await
        .map_err(|e| anyhow::anyhow!("failed to run migrations: {e}"))?;
    cmd_memory_with(&db, action).await
}

async fn cmd_memory_with(db: &Database, action: MemoryAction) -> Result<()> {
    match action {
        MemoryAction::Stats => {
            let store = LtmStore::new(db.clone());
            let s = store
                .stats()
                .await
                .map_err(|e| anyhow::anyhow!("memory stats failed: {e}"))?;
            println!("memories      : {}", s.total);
            println!("avg importance: {:.2}", s.avg_importance);
            println!("avg decay     : {:.2}", s.avg_decay_score);
            for (category, count) in &s.by_category {
                println!("  {category:<15}: {count}");
            }
        }
    }
    Ok(())
}

async fn cmd_cache(action: CacheAction) -> Result<()> {
    // The semantic cache is an in-process LRU local to each running gateway
    // (no Redis, no on-disk store — ADR 0008). A one-shot CLI invocation has no
    // shared cache to act on, so these commands report that rather than faking
    // persistence.
    match action {
        CacheAction::Clear => {
            println!("semantic cache is in-process (per running gateway) and not persisted.");
            println!("it is emptied automatically when the process exits — nothing to clear.");
        }
        CacheAction::Stats => {
            println!("semantic cache : in-process LRU, not persisted");
            println!(
                "hit threshold  : {:.2} cosine similarity",
                talon_memory::cache::SEMANTIC_CACHE_THRESHOLD
            );
        }
    }
    Ok(())
}

async fn cmd_doctor() -> Result<()> {
    println!("talon doctor — not yet implemented (Phase 7)");
    Ok(())
}

/// Resolve the LLM provider to use. Prefers the `[llm]` chain in
/// `~/.talon/config.toml` (written by `talon init`); a multi-entry chain
/// becomes a [`FallbackProvider`]. With no config it falls back to the
/// `TALON_LLM_PROVIDER`/`TALON_LLM_API_KEY` env path. Returns `Ok(None)` when a
/// key is required but missing — the caller prints guidance and exits cleanly.
/// Shared by `cmd_run` and `cmd_serve`.
fn resolve_provider() -> Result<Option<Arc<dyn LlmProvider>>> {
    let cfg = talon_home()
        .map(|p| LlmConfig::load(&p.join("config.toml")))
        .unwrap_or_default();

    if !cfg.is_empty() {
        let mut providers: Vec<Arc<dyn LlmProvider>> = Vec::with_capacity(cfg.chain.len());
        for choice in &cfg.chain {
            providers.push(build_provider_from_choice(choice)?);
        }
        let provider: Arc<dyn LlmProvider> = if providers.len() == 1 {
            providers.remove(0)
        } else {
            Arc::new(FallbackProvider::new(providers))
        };
        return Ok(Some(provider));
    }

    // Legacy env path — no configured chain.
    let provider_name =
        std::env::var("TALON_LLM_PROVIDER").unwrap_or_else(|_| "anthropic".to_string());
    let needs_api_key = matches!(provider_name.as_str(), "anthropic" | "openai");

    let key = if needs_api_key {
        let k = std::env::var("TALON_LLM_API_KEY")
            .ok()
            .or_else(|| load_provider_key(&provider_name))
            .unwrap_or_default();
        if k.is_empty() {
            println!(
                "No provider configured. Run `talon init`, set TALON_LLM_API_KEY, \
                 or use a key-less provider: TALON_LLM_PROVIDER=github-copilot"
            );
            return Ok(None);
        }
        k
    } else {
        String::new()
    };

    Ok(Some(build_single_provider(&provider_name, key)?))
}

/// Construct a provider for one chain entry, resolving its key from the
/// keychain and honoring model/base_url overrides.
fn build_provider_from_choice(choice: &ProviderChoice) -> Result<Arc<dyn LlmProvider>> {
    let preset = talon_llm::presets::find(&choice.provider)
        .with_context(|| format!("unknown provider '{}'", choice.provider))?;

    let key = if preset.needs_api_key() {
        load_provider_key(preset.name).unwrap_or_default()
    } else {
        String::new()
    };

    match preset.name {
        "github-copilot" => {
            Ok(Arc::new(GitHubCopilotProvider::new().map_err(|e| {
                anyhow::anyhow!("GitHub Copilot auth failed: {e}")
            })?))
        }
        "anthropic" => Ok(Arc::new(AnthropicProvider::new(key))),
        _ if preset.openai_compatible => {
            let base_url = choice.base_url.as_deref().unwrap_or(preset.base_url);
            let model = choice.model.as_deref().unwrap_or(preset.default_model);
            Ok(Arc::new(OpenAiCompatProvider::new(
                base_url.to_string(),
                key,
                model.to_string(),
            )))
        }
        other => anyhow::bail!("provider '{other}' is not supported in this build"),
    }
}

/// Construct a single provider from the legacy env path.
fn build_single_provider(provider_name: &str, api_key: String) -> Result<Arc<dyn LlmProvider>> {
    match provider_name {
        "github-copilot" | "copilot" => {
            Ok(Arc::new(GitHubCopilotProvider::new().map_err(|e| {
                anyhow::anyhow!("GitHub Copilot auth failed: {e}")
            })?))
        }
        // "anthropic" and anything else defaults to Anthropic.
        _ => Ok(Arc::new(AnthropicProvider::new(api_key))),
    }
}

async fn cmd_run(
    message: Option<String>,
    _config: Option<PathBuf>,
    gateway_flag: String,
    accessible: bool,
) -> Result<()> {
    let provider = match resolve_provider()? {
        Some(p) => p,
        None => return Ok(()),
    };

    let ctx = build_gateway_context(provider).await?;
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
    let gateway = select_gateway(&gateway_flag, &ctx, accessible, None)?;
    gateway.run().await.map_err(|e| anyhow::anyhow!("{e}"))
}

/// `[gateway]` settings from `~/.talon/config.toml` that the daemon needs.
struct GatewayConfig {
    http_addr: String,
    api_token: Option<String>,
}

/// Read `[gateway] http_addr` / `api_token` from the config file. Missing
/// file, section, or keys are all normal — defaults apply (and no token means
/// the web console fail-closes to "not mounted").
fn load_gateway_config() -> GatewayConfig {
    let table = talon_home()
        .ok()
        .map(|p| p.join("config.toml"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| toml::from_str::<toml::Table>(&raw).ok());
    let gateway = table.as_ref().and_then(|t| t.get("gateway"));
    let get_str = |key: &str| -> Option<String> {
        gateway
            .and_then(|g| g.get(key))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
    };
    GatewayConfig {
        http_addr: get_str("http_addr").unwrap_or_else(|| "127.0.0.1:7777".to_string()),
        api_token: get_str("api_token"),
    }
}

/// Build the foreground gateway selected by `--gateway`. "cli" and any unknown
/// value fall back to CLI. Shared by `cmd_run` and `cmd_serve`. `web` mounts
/// the console API on the HTTP gateway (only `cmd_serve` passes it).
fn select_gateway(
    gateway_flag: &str,
    ctx: &Arc<GatewayContext>,
    accessible: bool,
    web: Option<WebState>,
) -> Result<Arc<dyn Gateway>> {
    let gateway: Arc<dyn Gateway> = match gateway_flag {
        "http" => {
            let addr = load_gateway_config()
                .http_addr
                .parse()
                .context("invalid [gateway] http_addr")?;
            let mut gw = HttpGateway::new(Arc::clone(ctx), addr);
            if let Some(web) = web {
                gw = gw.with_web(web);
            }
            Arc::new(gw)
        }
        "tui" => Arc::new(TuiGateway::new(Arc::clone(ctx), accessible, "talon")),
        #[cfg(feature = "telegram")]
        "telegram" => {
            let gw = talon_gateway::telegram::TelegramGateway::from_env(Arc::clone(ctx))
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            Arc::new(gw)
        }
        _ => {
            let mode = if accessible {
                talon_gateway::RenderMode::Accessible
            } else {
                talon_gateway::RenderMode::Plain
            };
            Arc::new(CliGateway::new(Arc::clone(ctx), mode))
        }
    };
    Ok(gateway)
}

async fn build_gateway_context(provider: Arc<dyn LlmProvider>) -> Result<GatewayContext> {
    let mut ctx = GatewayContext::new(provider);

    // Register built-in tools.
    ctx = ctx.with_tool(Arc::new(EchoTool));
    ctx = ctx.with_tool(Arc::new(ReadFileTool));

    // Phase 5 web tools (Safe) — backend chains assembled from [tools.web] in
    // ~/.talon/config.toml (defaults: Brave→DDG search, native-only fetch).
    let web_cfg = WebConfig::load(
        &talon_home()
            .map(|p| p.join("config.toml"))
            .unwrap_or_default(),
    );
    ctx = ctx.with_tool(timeouts::with_timeout(
        WebSearchTool::with_backends(web_cfg.build_search_chain()),
        timeouts::WEB_TIMEOUT_SECS,
    ));
    ctx = ctx.with_tool(timeouts::with_timeout(
        WebExtractTool::with_backends(web_cfg.build_fetch_chain()),
        timeouts::WEB_TIMEOUT_SECS,
    ));

    // Phase 5 browser tool — experimental, only when built with `--features browser`.
    #[cfg(feature = "browser")]
    {
        use talon_tools::browser::{BrowserPool, BrowserTool};
        let pool = Arc::new(BrowserPool::default());
        ctx = ctx.with_tool(timeouts::with_timeout(
            BrowserTool::new(pool),
            timeouts::BROWSER_TIMEOUT_SECS,
        ));
    }

    // Phase 5 MCP servers from ~/.talon/mcp_servers.toml. Each adapted tool is
    // already wrapped with the MCP timeout inside `adapt_server`.
    let mcp_cfg = McpServersConfig::load(&McpServersConfig::default_path()).unwrap_or_else(|e| {
        tracing::warn!("mcp config: {e}");
        McpServersConfig::default()
    });
    for entry in mcp_cfg.server {
        let transport = match entry.to_transport() {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("mcp '{}': {e}", entry.name);
                continue;
            }
        };
        match McpClient::connect(transport).await {
            Ok(client) => match adapt_server(Arc::new(client)).await {
                Ok(tools) => {
                    tracing::info!("mcp '{}': registered {} tool(s)", entry.name, tools.len());
                    for tool in tools {
                        ctx = ctx.with_tool(tool);
                    }
                }
                Err(e) => tracing::warn!("mcp '{}' tools/list failed: {e}", entry.name),
            },
            Err(e) => tracing::warn!("mcp '{}' connect failed: {e}", entry.name),
        }
    }

    // Phase 6 WASM skills from ~/.talon/skills/ (opt-in `skills` feature). Each
    // loaded skill is registered as a tool at startup. Live hot-reload into the
    // running dispatcher is a follow-up; the SkillStore itself supports reload.
    #[cfg(feature = "skills")]
    if let Ok(skills_dir) = talon_home().map(|p| p.join("skills")) {
        match talon_plugins::PluginHost::new() {
            Ok(host) => {
                let store = talon_plugins::SkillStore::new(Arc::new(host), skills_dir);
                let tools = store.tools();
                if !tools.is_empty() {
                    tracing::info!("skills: loaded {} skill(s)", tools.len());
                }
                for tool in tools {
                    ctx = ctx.with_tool(tool);
                }
            }
            Err(e) => tracing::warn!("skills: plugin host init failed: {e}"),
        }
    }

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
        // Scheduling surface: create/list/delete cron jobs (NeedsApproval).
        let cron_store = Arc::new(CronStore::new(Arc::clone(db)));
        ctx = ctx.with_tool(Arc::new(CronJobTool::new(cron_store)));
        ctx = ctx.with_db(Arc::clone(db));
    }

    Ok(ctx)
}

// ── Daemon (`talon serve`) ────────────────────────────────────────────────────

const DEFAULT_SCHEDULER_TICK_SECS: u64 = 30;
/// Bounded grace for in-flight cron jobs to finish after a shutdown signal.
const SHUTDOWN_GRACE_SECS: u64 = 10;

/// Where a scheduled job's output is routed (SPEC §4.2 deliver target).
#[derive(Debug, Clone, PartialEq, Eq)]
enum DeliverTarget {
    /// Back to the session/channel that created the job (default).
    Origin,
    /// The local CLI/log only — never pushed to a remote channel.
    Local,
    /// Fan out to every connected gateway.
    All,
    /// A specific platform conversation: `platform:chat[:thread]`.
    Platform {
        platform: String,
        chat_id: String,
        thread_id: Option<String>,
    },
}

/// Parse a `deliver_to` string into a [`DeliverTarget`]. Pure and testable.
/// Grammar: `origin` | `local` | `all` | `platform:chat[:thread]`.
fn parse_deliver_target(s: &str) -> DeliverTarget {
    match s.trim() {
        "" | "origin" => DeliverTarget::Origin,
        "local" => DeliverTarget::Local,
        "all" => DeliverTarget::All,
        other => {
            let mut parts = other.splitn(3, ':');
            let platform = parts.next().unwrap_or_default().to_string();
            match parts.next() {
                Some(chat_id) if !platform.is_empty() && !chat_id.is_empty() => {
                    DeliverTarget::Platform {
                        platform,
                        chat_id: chat_id.to_string(),
                        thread_id: parts.next().map(str::to_string).filter(|t| !t.is_empty()),
                    }
                }
                // Anything we can't parse routes to the origin rather than dropping.
                _ => DeliverTarget::Origin,
            }
        }
    }
}

/// The concrete [`JobRunner`] for the daemon: builds a fresh agent per job from
/// the shared [`GatewayContext`], runs the prompt to completion, accumulates the
/// assistant text, and routes it to the job's `deliver_to` target.
///
/// Unattended-safety: any tool that asks for approval mid-run is **denied** —
/// a scheduled job never silently runs a `NeedsApproval`/`Dangerous` tool.
/// Phase 4's scope wizard pre-grants a per-job tool allowlist; until then the
/// only safe default is to refuse escalation.
struct TalonJobRunner {
    ctx: Arc<GatewayContext>,
    /// Web console approval path (§4.4 A). `None` → immediate deny, as before.
    approvals: Option<ApprovalBroker>,
    /// Web console live feed; approval escalations are announced here.
    events: Option<broadcast::Sender<RunEvent>>,
    /// JIT secret resolution (criteria 10–11). `None` → prompts pass through
    /// unchanged (no secrets subsystem configured).
    resolver: Option<Arc<talon_secrets::SecretResolver>>,
}

/// How long an out-of-scope escalation waits for a ✅/❌ from the console
/// before it is denied ("skipped: out of granted scope").
const APPROVAL_TIMEOUT_SECS: u64 = 120;

impl JobRunner for TalonJobRunner {
    fn run(&self, job: CronJob) -> Pin<Box<dyn Future<Output = JobOutcome> + Send + '_>> {
        let ctx = Arc::clone(&self.ctx);
        let approvals = self.approvals.clone();
        let events = self.events.clone();
        let resolver = self.resolver.clone();
        Box::pin(async move {
            let (tx, mut rx) = mpsc::channel::<AgentEvent>(64);
            // Run unattended under the job's pre-authorized scope: out-of-scope
            // tool calls escalate (and are denied below) rather than running.
            let mut agent = ctx
                .build_agent(tx)
                .with_unattended_scope(job.granted_scope.clone());

            let session_id = job.session_id.clone();
            let deliver_to = job.deliver_to.clone();
            let job_id = job.id.clone();
            let job_label = job.name.clone().unwrap_or_else(|| job.id.clone());

            // JIT secret resolution (criteria 10–11): the resolved prompt goes
            // to the agent only; every resolved value is registered for
            // redaction for the lifetime of this run. An unresolvable
            // reference aborts the run before the LLM is ever called, with an
            // error naming the reference — never a value.
            let mut _redaction_guards = Vec::new();
            let prompt = match &resolver {
                Some(r) => match r.resolve_all(&job.prompt).await {
                    Ok(resolved) => {
                        for (name, value) in &resolved.values {
                            _redaction_guards.push(
                                talon_secrets::redact::global().register(name, value.expose()),
                            );
                        }
                        resolved.text
                    }
                    Err(e) => {
                        tracing::error!(
                            job = %job_label,
                            error = %e,
                            "secret resolution failed — run aborted before dispatch"
                        );
                        return JobOutcome::failed_with(format!("secret resolution failed: {e}"));
                    }
                },
                None => job.prompt.clone(),
            };

            // Drive the agent on its own task so we can drain events concurrently;
            // the channel closes when the agent (and its tx) drop.
            let handle = tokio::spawn(async move {
                agent
                    .run(&session_id, prompt)
                    .await
                    .map_err(|e| e.to_string())
            });

            let mut output = String::new();
            let mut succeeded = false;
            let mut error: Option<String> = None;
            while let Some(event) = rx.recv().await {
                match event {
                    AgentEvent::Text { content } => {
                        if !output.is_empty() {
                            output.push('\n');
                        }
                        output.push_str(&content);
                    }
                    AgentEvent::ApprovalRequested {
                        call_id,
                        tool_name,
                        args,
                        tx,
                        ..
                    } => match &approvals {
                        // Web console attached: park the approval (§4.4 A) —
                        // resolved by POST /approvals/{call_id} or denied on
                        // timeout. The run blocks on its own task meanwhile.
                        Some(broker) => {
                            tracing::warn!(
                                job = %job_label,
                                tool = %tool_name,
                                call_id = %call_id,
                                "out-of-scope tool call — escalating to web console"
                            );
                            broker.register(
                                PendingApproval::new(
                                    call_id.clone(),
                                    Some(job_id.clone()),
                                    tool_name.clone(),
                                    args.clone(),
                                ),
                                tx,
                            );
                            if let Some(ev) = &events {
                                let _ = ev.send(RunEvent::ApprovalPending {
                                    call_id: call_id.clone(),
                                    job_id: Some(job_id.clone()),
                                    tool: tool_name,
                                    args,
                                });
                            }
                            let broker = broker.clone();
                            let ev = events.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_secs(APPROVAL_TIMEOUT_SECS))
                                    .await;
                                if broker.deny_if_pending(&call_id) {
                                    tracing::warn!(
                                        call_id = %call_id,
                                        "approval timed out — denied (out of granted scope)"
                                    );
                                    if let Some(ev) = ev {
                                        let _ = ev.send(RunEvent::ApprovalResolved {
                                            call_id,
                                            approved: false,
                                        });
                                    }
                                }
                            });
                        }
                        None => {
                            tracing::warn!(
                                job = %job_label,
                                tool = %tool_name,
                                "scheduled job requested tool approval — denying (unattended)"
                            );
                            let _ = tx.send(false);
                        }
                    },
                    AgentEvent::Completed => succeeded = true,
                    AgentEvent::Failed(msg) => {
                        tracing::error!(job = %job_label, error = %msg, "scheduled job failed");
                        succeeded = false;
                        error = Some(msg);
                    }
                    _ => {}
                }
            }

            // Surface a panic/await error from the agent task as a failure.
            if let Ok(Err(e)) = handle.await {
                tracing::error!(job = %job_label, error = %e, "agent run errored");
                succeeded = false;
                error = Some(e);
            }

            // Scrub the outcome at the choke point (criterion 10): these
            // fields become cron_runs.output / .error / last_output, so no
            // resolved value may survive past this line. Guards are still
            // alive here — scrub must precede their drop.
            let registry = talon_secrets::redact::global();
            let output = registry.scrub_owned(output);
            let error = error.map(|e| registry.scrub_owned(e));

            let final_output = (!output.is_empty()).then_some(output.clone());
            deliver(&job_label, &deliver_to, final_output.as_deref()).await;

            if succeeded {
                JobOutcome::ok(final_output)
            } else {
                match error {
                    Some(e) => JobOutcome::failed_with(e),
                    None => JobOutcome::failed(),
                }
            }
        })
    }
}

/// Build the daemon's secret resolver: `env` always; the builtin vault when
/// it is bootstrapped AND unlockable without a prompt (keychain or
/// `TALON_MASTER_KEY` — a daemon never blocks on a passphrase). A locked
/// vault is logged loudly; `{{secret:NAME}}` refs then fail their runs with
/// an actionable error (criterion 2), never silently.
fn build_secret_resolver(db: Option<Arc<Database>>) -> Arc<talon_secrets::SecretResolver> {
    use talon_secrets::{BuiltinVault, EnvProvider, MasterKeyStore, OsKeychain, SecretResolver};

    let mut resolver = SecretResolver::new();
    resolver.register(Arc::new(EnvProvider));

    if let (Some(db), Ok(home)) = (db, talon_home()) {
        let keychain = OsKeychain;
        let key_store = MasterKeyStore::new(&home, &keychain);
        match key_store.is_bootstrapped() {
            Ok(true) => {
                let env_value = std::env::var(talon_secrets::ENV_VAR).ok();
                match key_store.unlock(env_value.as_deref(), None) {
                    Ok(master) => {
                        resolver.register(Arc::new(BuiltinVault::new(db, master)));
                        tracing::info!("builtin secret vault unlocked");
                    }
                    Err(e) => tracing::warn!(
                        "vault locked: {e} — jobs using {{{{secret:NAME}}}} will fail until unlocked"
                    ),
                }
            }
            Ok(false) => {
                tracing::debug!("no vault master key — builtin secrets disabled (run `talon init`)");
            }
            Err(e) => tracing::warn!("vault state check failed: {e}"),
        }
    }
    Arc::new(resolver)
}

/// Route a finished job's output to its target. Phase 3 logs the delivery; live
/// remote sends (Telegram et al.) are wired in a follow-up live-smoke pass.
async fn deliver(job_label: &str, deliver_to: &str, output: Option<&str>) {
    let target = parse_deliver_target(deliver_to);
    let preview = output.unwrap_or("(no output)");
    tracing::info!(
        job = %job_label,
        target = ?target,
        chars = preview.len(),
        "cron job delivered"
    );
}

async fn cmd_serve(gateway_flag: String, accessible: bool) -> Result<()> {
    let provider = match resolve_provider()? {
        Some(p) => p,
        None => return Ok(()),
    };

    let ctx = Arc::new(build_gateway_context(provider).await?);

    let db = ctx
        .db
        .clone()
        .context("serve requires a database — none was opened")?;
    let store = CronStore::new(Arc::clone(&db));
    let run_store = RunStore::new(db);

    let tick_secs = std::env::var("TALON_SCHEDULER_TICK_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_SCHEDULER_TICK_SECS);

    // Web console plumbing: live event feed + approval broker. The console
    // itself only mounts when [gateway] api_token is configured (fail closed).
    let gateway_cfg = load_gateway_config();
    let (event_tx, _) = broadcast::channel::<RunEvent>(EVENT_CHANNEL_CAP);
    let approvals = ApprovalBroker::new();

    let runner = Arc::new(TalonJobRunner {
        ctx: Arc::clone(&ctx),
        approvals: gateway_cfg.api_token.is_some().then(|| approvals.clone()),
        events: Some(event_tx.clone()),
        resolver: Some(build_secret_resolver(ctx.db.clone())),
    });
    let scheduler = Scheduler::new(store.clone(), runner)
        .with_tick(Duration::from_secs(tick_secs))
        .with_run_store(run_store.clone())
        .with_events(event_tx.clone());
    let sched_handle = scheduler.handle();

    let web_state = match &gateway_cfg.api_token {
        Some(token) => Some(
            WebState::new(
                Arc::clone(&ctx),
                store,
                run_store,
                sched_handle,
                event_tx,
                approvals,
                token,
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?,
        ),
        None => {
            tracing::warn!(
                "no [gateway] api_token in config.toml — web console API not mounted \
                 (run `talon init` to generate one)"
            );
            None
        }
    };

    let cancel = CancellationToken::new();
    let tracker = TaskTracker::new();

    // Scheduler tick-loop.
    let scheduler_handle = {
        let cancel = cancel.clone();
        let tracker = tracker.clone();
        tokio::spawn(async move { scheduler.run(cancel, tracker).await })
    };

    // Foreground gateway — aborted on shutdown; the scheduler is what we drain.
    // The web console rides the HTTP gateway: foreground when --gateway http,
    // otherwise as an extra background server so `talon serve` always exposes it.
    let (foreground_web, background_web) = if gateway_flag == "http" {
        (web_state, None)
    } else {
        (None, web_state)
    };
    let gateway = select_gateway(&gateway_flag, &ctx, accessible, foreground_web)?;
    let gateway_handle =
        tokio::spawn(async move { gateway.run().await.map_err(|e| e.to_string()) });

    let _web_handle = background_web.map(|web| {
        let addr = gateway_cfg.http_addr.clone();
        let ctx = Arc::clone(&ctx);
        tokio::spawn(async move {
            match addr.parse() {
                Ok(addr) => {
                    let gw = HttpGateway::new(ctx, addr).with_web(web);
                    if let Err(e) = gw.run().await {
                        tracing::error!(error = %e, "web console HTTP server failed");
                    }
                }
                Err(e) => tracing::error!(error = %e, "invalid [gateway] http_addr"),
            }
        })
    });

    tracing::info!(
        gateway = %gateway_flag,
        tick_secs,
        "talon serve started — scheduler + gateway running"
    );

    // Block until a shutdown signal or the gateway exits on its own.
    tokio::select! {
        _ = shutdown_signal() => tracing::info!("shutdown signal received"),
        res = gateway_handle => match res {
            Ok(Ok(())) => tracing::info!("gateway exited"),
            Ok(Err(e)) => tracing::error!(error = %e, "gateway errored"),
            Err(e) => tracing::error!(error = %e, "gateway task join failed"),
        },
    }

    // Stop accepting new ticks, then let in-flight jobs drain within the grace.
    cancel.cancel();
    let _ = scheduler_handle.await;
    tracker.close();
    match tokio::time::timeout(Duration::from_secs(SHUTDOWN_GRACE_SECS), tracker.wait()).await {
        Ok(()) => tracing::info!("all in-flight jobs drained"),
        Err(_) => tracing::warn!(
            grace_secs = SHUTDOWN_GRACE_SECS,
            "drain grace elapsed — some jobs may have been cut short"
        ),
    }

    Ok(())
}

/// Resolve when the process should begin a graceful shutdown: Ctrl-C on every
/// platform, plus SIGTERM on Unix (the signal a service manager sends).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => tracing::error!(error = %e, "failed to install SIGTERM handler"),
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn init_tracing(level: &str) -> Result<()> {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_new(level).or_else(|_| EnvFilter::try_new("info"))?;

    // All formatted output passes the redaction registry (criterion 10) —
    // resolved secret values never reach the terminal.
    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(logging::ScrubStderr)
        .init();
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
# Web console bearer token — generated by `talon init`; /api/v1 does not
# mount without it. v1 SSE auth uses ?token= (EventSource cannot set headers);
# keep the bind localhost unless you terminate TLS in front.
# api_token = "talon_..."
"#
}

/// Store a provider's API key in the OS keychain under `<provider>-api-key`.
fn store_provider_key(provider: &str, key: &str) -> Result<()> {
    use keyring::Entry;
    let entry = Entry::new("talon", &format!("{provider}-api-key"))
        .context("failed to create keyring entry")?;
    entry
        .set_password(key)
        .context("failed to store password")?;
    Ok(())
}

/// Load a provider's API key from the OS keychain, if present and non-empty.
fn load_provider_key(provider: &str) -> Option<String> {
    use keyring::Entry;
    Entry::new("talon", &format!("{provider}-api-key"))
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
    fn cli_parses_serve_defaults_to_http() -> Result<()> {
        let cli = Cli::try_parse_from(["talon", "serve"])?;
        match cli.command {
            Some(Commands::Serve { gateway }) => assert_eq!(gateway, "http"),
            _ => panic!("expected serve subcommand"),
        }
        Ok(())
    }

    #[test]
    fn cli_parses_serve_with_gateway() -> Result<()> {
        let cli = Cli::try_parse_from(["talon", "serve", "--gateway", "telegram"])?;
        match cli.command {
            Some(Commands::Serve { gateway }) => assert_eq!(gateway, "telegram"),
            _ => panic!("expected serve subcommand"),
        }
        Ok(())
    }

    // ── parse_deliver_target ────────────────────────────────────────────────────

    #[test]
    fn deliver_target_empty_and_origin_are_origin() {
        assert_eq!(parse_deliver_target(""), DeliverTarget::Origin);
        assert_eq!(parse_deliver_target("origin"), DeliverTarget::Origin);
        assert_eq!(parse_deliver_target("  origin  "), DeliverTarget::Origin);
    }

    #[test]
    fn deliver_target_local_and_all() {
        assert_eq!(parse_deliver_target("local"), DeliverTarget::Local);
        assert_eq!(parse_deliver_target("all"), DeliverTarget::All);
    }

    #[test]
    fn deliver_target_platform_chat() {
        assert_eq!(
            parse_deliver_target("telegram:12345"),
            DeliverTarget::Platform {
                platform: "telegram".to_string(),
                chat_id: "12345".to_string(),
                thread_id: None,
            }
        );
    }

    #[test]
    fn deliver_target_platform_chat_thread() {
        assert_eq!(
            parse_deliver_target("telegram:12345:67"),
            DeliverTarget::Platform {
                platform: "telegram".to_string(),
                chat_id: "12345".to_string(),
                thread_id: Some("67".to_string()),
            }
        );
    }

    #[test]
    fn deliver_target_malformed_falls_back_to_origin() {
        // Bare token with no chat id is not a valid platform spec.
        assert_eq!(parse_deliver_target("telegram"), DeliverTarget::Origin);
        assert_eq!(parse_deliver_target("telegram:"), DeliverTarget::Origin);
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

    // ── provider chain construction (W7) ──────────────────────────────────────

    #[test]
    fn build_provider_from_choice_rejects_unknown() {
        let choice = ProviderChoice::new("not-a-real-provider");
        assert!(build_provider_from_choice(&choice).is_err());
    }

    #[test]
    fn build_provider_from_choice_builds_anthropic() {
        // AnthropicProvider::new does no network/auth — construction must succeed.
        let choice = ProviderChoice::new("anthropic");
        assert!(build_provider_from_choice(&choice).is_ok());
    }

    #[test]
    fn build_provider_from_choice_builds_openai_compatible() {
        // OpenAiCompatProvider::new only builds an HTTP client — no network.
        let choice = ProviderChoice::new("openrouter");
        assert!(build_provider_from_choice(&choice).is_ok());
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
        let db = Database::open(":memory:").map_err(|e| anyhow::anyhow!("{e}"))?;
        db.init_schema().await.map_err(|e| anyhow::anyhow!("{e}"))?;
        cmd_memory_with(&db, MemoryAction::Stats).await
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

    /// Phase 5 web tools register into a GatewayContext (timeout-wrapped) and
    /// report their names through the wrapper. Hermetic — no DB/network/env.
    #[test]
    fn phase5_web_tools_register() {
        let provider = Arc::new(AnthropicProvider::new("dummy".to_string()));
        let ctx = GatewayContext::new(provider)
            .with_tool(timeouts::with_timeout(
                WebSearchTool::new(),
                timeouts::WEB_TIMEOUT_SECS,
            ))
            .with_tool(timeouts::with_timeout(
                WebExtractTool::new(),
                timeouts::WEB_TIMEOUT_SECS,
            ));
        let names: Vec<String> = ctx.tools.iter().map(|t| t.name().to_string()).collect();
        assert!(names.iter().any(|n| n == "web_search"), "names: {names:?}");
        assert!(names.iter().any(|n| n == "web_extract"), "names: {names:?}");
    }
}
