# Logging & Observability

> **Status:** ✅ Complete
> **Category:** DevOps
> **Last corrected:** dogfood pass 3

---

## 1. Tracing Stack

```toml
[dependencies]
tracing            = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json", "fmt"] }
tracing-appender   = "0.2"
tracing-opentelemetry = "0.22"   # optional: OTEL export
opentelemetry      = "0.22"
opentelemetry-otlp = "0.15"
```

---

## 2. Subscriber Init

```rust
pub fn init_tracing(cfg: &ObservabilityConfig) -> Result<(), Box<dyn std::error::Error>> {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&cfg.log_level));

    let fmt_layer = if cfg.json_logs {
        tracing_subscriber::fmt::layer()
            .json()
            .with_target(true)
            .with_thread_ids(true)
            .with_span_list(true)
            .boxed()
    } else {
        tracing_subscriber::fmt::layer()
            .pretty()
            .with_target(false)
            .boxed()
    };

    let file_appender = cfg.log_file.as_ref().map(|path| {
        let appender = tracing_appender::rolling::daily(
            path.parent().unwrap(),
            path.file_name().unwrap(),
        );
        tracing_appender::non_blocking(appender).0
    });

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    if let Some(writer) = file_appender {
        registry
            .with(tracing_subscriber::fmt::layer().json().with_writer(writer))
            .init();
    } else {
        registry.init();
    }

    Ok(())
}
```

---

## 3. Span Conventions

```rust
// Agent turn: instrument the whole async function
#[tracing::instrument(
    skip(self, messages),
    fields(
        session_id = %session_id,
        model = %self.config.model,
        turn_num,
    )
)]
pub async fn run_turn(
    &self,
    session_id: Uuid,
    messages: Vec<Message>,
) -> Result<Turn, AgentError> {
    let span = tracing::Span::current();
    span.record("turn_num", self.turn_count.load(Ordering::Relaxed));
    // ...
}

// Tool execution
#[tracing::instrument(
    skip(ctx),
    fields(tool = %name, call_id = %ctx.call_id)
)]
pub async fn execute_tool(name: &str, ctx: ToolContext) -> Result<ToolResult, ToolError> {
    // ...
}
```

---

## 4. Structured Log Fields

| Field | Type | Description |
|-------|------|-------------|
| `session_id` | UUID | Conversation session |
| `turn_num` | u32 | Turn index in session |
| `model` | str | LLM model used |
| `tool` | str | Tool name |
| `call_id` | str | Tool call ID from LLM |
| `tokens_in` | u32 | Input tokens (from Usage delta) |
| `tokens_out` | u32 | Output tokens |
| `latency_ms` | u64 | Tool/LLM call latency |
| `provider` | str | LLM provider ID |
| `gateway` | str | Channel that sent message |
| `job_id` | UUID | [Cron job](../04_Core_Features/33_Cron_Scheduler.md) ID (if cron context) |

---

## 5. Metrics (Optional Prometheus)

```toml
[dependencies]
metrics            = "0.22"
metrics-exporter-prometheus = "0.13"
```

```rust
// In agent loop
metrics::counter!("talon.turns.total", "model" => model.clone()).increment(1);
metrics::histogram!("talon.turn.latency_ms", "model" => model).record(elapsed_ms);
metrics::counter!("talon.tool.calls", "tool" => tool_name.clone()).increment(1);
metrics::counter!("talon.tool.errors", "tool" => tool_name).increment(1);
metrics::counter!("talon.tokens.in", "model" => model).increment(tokens_in as u64);
metrics::counter!("talon.tokens.out", "model" => model).increment(tokens_out as u64);

// Expose /metrics endpoint
let builder = PrometheusBuilder::new();
builder.install_recorder()?;
// in axum router:
app.route("/metrics", get(|| async { metrics_exporter_prometheus::render() }));
```

---

## 6. RUST_LOG Reference

```bash
# Default: info level for talon, warn for deps
RUST_LOG=talon=info,reqwest=warn,hyper=warn

# Debug tool execution only
RUST_LOG=talon_tools=debug,talon=info

# Full trace (very verbose)
RUST_LOG=trace

# JSON output for log aggregation
TALON_JSON_LOGS=true RUST_LOG=talon=info ./talon
```

---

## 7. Health Check Endpoint

```rust
// Exposed at GET /health via axum gateway
async fn health_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let db_ok = state.memory.ping().await.is_ok();
    let providers: Vec<_> = state.llm.list_providers()
        .iter()
        .map(|p| serde_json::json!({ "id": p, "ok": true }))
        .collect();

    let status = if db_ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };

    (status, Json(serde_json::json!({
        "status": if db_ok { "ok" } else { "degraded" },
        "version": env!("CARGO_PKG_VERSION"),
        "database": db_ok,
        "providers": providers,
        "uptime_secs": state.uptime.elapsed().as_secs(),
    })))
}
```

---

## 8. Log Rotation

```rust
// Daily rolling log files
let file_appender = tracing_appender::rolling::Builder::new()
    .rotation(tracing_appender::rolling::Rotation::DAILY)
    .filename_prefix("talon")
    .filename_suffix("log")
    .max_log_files(14)  // keep 2 weeks
    .build("/var/log/talon")?;
```

For production: ship logs to Loki via `alloy` (Grafana Agent), query with Grafana. No proprietary SaaS required.
---

## Related Documents

### See Also
- [Error Handling Strategy](../06_Concurrency/54_Error_Handling_Strategy.md)
- [CI/CD Pipeline](62_CI_CD_Pipeline.md)

