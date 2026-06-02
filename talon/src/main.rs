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
use talon_core::scheduler::{JobOutcome, JobRunner, Scheduler};
use talon_core::tools::{Tool, ToolContext, ToolResult};
use talon_gateway::{Gateway, GatewayContext, cli::CliGateway, http::HttpGateway, tui::TuiGateway};
use talon_llm::{AnthropicProvider, GitHubCopilotProvider, LlmProvider};
use talon_memory::{CronJob, CronStore, Database, LtmStore, SqliteStore};
use talon_tools::mcp::{McpClient, McpServersConfig, adapt_server};
use talon_tools::web::WebConfig;
use talon_tools::{CronJobTool, SessionSearchTool, WebExtractTool, WebSearchTool, timeouts};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

mod cron_cli;
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

/// Resolve the LLM provider name and (possibly empty) API key from env +
/// keychain. Returns `Ok(None)` when a key is required but missing — the caller
/// prints guidance and exits cleanly. Shared by `cmd_run` and `cmd_serve`.
fn resolve_provider_and_key() -> Result<Option<(String, String)>> {
    let provider_name =
        std::env::var("TALON_LLM_PROVIDER").unwrap_or_else(|_| "anthropic".to_string());

    // Key-less providers (github-copilot, claude-code) don't need TALON_LLM_API_KEY.
    let needs_api_key = matches!(provider_name.as_str(), "anthropic" | "openai");

    if !needs_api_key {
        return Ok(Some((provider_name, String::new())));
    }

    let key = std::env::var("TALON_LLM_API_KEY")
        .ok()
        .or_else(|| load_provider_key(&provider_name))
        .unwrap_or_default();
    if key.is_empty() {
        println!(
            "API key not configured. Run `talon init`, set TALON_LLM_API_KEY, \
             or use a key-less provider: TALON_LLM_PROVIDER=github-copilot"
        );
        return Ok(None);
    }
    Ok(Some((provider_name, key)))
}

async fn cmd_run(
    message: Option<String>,
    _config: Option<PathBuf>,
    gateway_flag: String,
    accessible: bool,
) -> Result<()> {
    let (provider_name, api_key) = match resolve_provider_and_key()? {
        Some(pair) => pair,
        None => return Ok(()),
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
    let gateway = select_gateway(&gateway_flag, &ctx, accessible)?;
    gateway.run().await.map_err(|e| anyhow::anyhow!("{e}"))
}

/// Build the foreground gateway selected by `--gateway`. "cli" and any unknown
/// value fall back to CLI. Shared by `cmd_run` and `cmd_serve`.
fn select_gateway(
    gateway_flag: &str,
    ctx: &Arc<GatewayContext>,
    accessible: bool,
) -> Result<Arc<dyn Gateway>> {
    let gateway: Arc<dyn Gateway> = match gateway_flag {
        "http" => {
            let addr = "127.0.0.1:7777".parse().context("invalid HTTP addr")?;
            Arc::new(HttpGateway::new(Arc::clone(ctx), addr))
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
}

impl JobRunner for TalonJobRunner {
    fn run(&self, job: CronJob) -> Pin<Box<dyn Future<Output = JobOutcome> + Send + '_>> {
        let ctx = Arc::clone(&self.ctx);
        Box::pin(async move {
            let (tx, mut rx) = mpsc::channel::<AgentEvent>(64);
            // Run unattended under the job's pre-authorized scope: out-of-scope
            // tool calls escalate (and are denied below) rather than running.
            let mut agent = ctx
                .build_agent(tx)
                .with_unattended_scope(job.granted_scope.clone());

            let session_id = job.session_id.clone();
            let prompt = job.prompt.clone();
            let deliver_to = job.deliver_to.clone();
            let job_label = job.name.clone().unwrap_or_else(|| job.id.clone());

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
            while let Some(event) = rx.recv().await {
                match event {
                    AgentEvent::Text { content } => {
                        if !output.is_empty() {
                            output.push('\n');
                        }
                        output.push_str(&content);
                    }
                    AgentEvent::ApprovalRequested { tx, tool_name, .. } => {
                        tracing::warn!(
                            job = %job_label,
                            tool = %tool_name,
                            "scheduled job requested tool approval — denying (unattended)"
                        );
                        let _ = tx.send(false);
                    }
                    AgentEvent::Completed => succeeded = true,
                    AgentEvent::Failed(msg) => {
                        tracing::error!(job = %job_label, error = %msg, "scheduled job failed");
                        succeeded = false;
                    }
                    _ => {}
                }
            }

            // Surface a panic/await error from the agent task as a failure.
            if let Ok(Err(e)) = handle.await {
                tracing::error!(job = %job_label, error = %e, "agent run errored");
                succeeded = false;
            }

            let final_output = (!output.is_empty()).then_some(output.clone());
            deliver(&job_label, &deliver_to, final_output.as_deref()).await;

            if succeeded {
                JobOutcome::ok(final_output)
            } else {
                JobOutcome::failed()
            }
        })
    }
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
    let (provider_name, api_key) = match resolve_provider_and_key()? {
        Some(pair) => pair,
        None => return Ok(()),
    };

    let ctx = Arc::new(build_gateway_context(&provider_name, api_key).await?);

    let db = ctx
        .db
        .clone()
        .context("serve requires a database — none was opened")?;
    let store = CronStore::new(db);

    let tick_secs = std::env::var("TALON_SCHEDULER_TICK_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_SCHEDULER_TICK_SECS);

    let runner = Arc::new(TalonJobRunner {
        ctx: Arc::clone(&ctx),
    });
    let scheduler = Scheduler::new(store, runner).with_tick(Duration::from_secs(tick_secs));

    let cancel = CancellationToken::new();
    let tracker = TaskTracker::new();

    // Scheduler tick-loop.
    let scheduler_handle = {
        let cancel = cancel.clone();
        let tracker = tracker.clone();
        tokio::spawn(async move { scheduler.run(cancel, tracker).await })
    };

    // Foreground gateway — aborted on shutdown; the scheduler is what we drain.
    let gateway = select_gateway(&gateway_flag, &ctx, accessible)?;
    let gateway_handle =
        tokio::spawn(async move { gateway.run().await.map_err(|e| e.to_string()) });

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
