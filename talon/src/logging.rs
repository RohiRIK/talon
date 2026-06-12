//! Logging stack (criteria 16–18, 20).
//!
//! Layered subscriber, redaction on every sink:
//!
//! - stderr fmt (pretty by default, `[logging] format = "json"` switches) —
//!   unchanged default look
//! - JSON daily-rotated file at `~/.talon/logs/talon.log.YYYY-MM-DD`
//!   (`[logging] file = true`, the default)
//! - in-memory ring buffer feeding `GET /api/v1/logs/tail`
//! - OTLP span export behind the `otel` feature (`[otel] endpoint`)
//!
//! Every writer is wrapped in [`ScrubWriter`], so a resolved secret value
//! never reaches a terminal, a log file, or the console tail.

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;

use talon_gateway::web::logs::{LogLine, LogRing};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

// ── Redaction writers ────────────────────────────────────────────────────────

/// Wrap any `MakeWriter` so each chunk passes the redaction registry.
pub struct ScrubMakeWriter<M>(pub M);

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for ScrubMakeWriter<M> {
    type Writer = ScrubWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        ScrubWriter(self.0.make_writer())
    }
}

/// `MakeWriter` for stderr with redaction applied per write.
pub struct ScrubStderr;

impl<'a> MakeWriter<'a> for ScrubStderr {
    type Writer = ScrubWriter<io::Stderr>;

    fn make_writer(&'a self) -> Self::Writer {
        ScrubWriter(io::stderr())
    }
}

/// Writer adapter: scrubs UTF-8 chunks through the redaction registry.
/// Non-UTF-8 chunks pass through untouched (fmt output is always UTF-8).
pub struct ScrubWriter<W: Write>(pub W);

impl<W: Write> Write for ScrubWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match std::str::from_utf8(buf) {
            Ok(s) => self
                .0
                .write_all(talon_secrets::redact::scrub(s).as_bytes())?,
            Err(_) => self.0.write_all(buf)?,
        }
        // Report the input as fully consumed — the caller's buffer length,
        // not the (possibly different) scrubbed length.
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

// ── Ring-buffer layer (criterion 20) ─────────────────────────────────────────

/// Feeds formatted events into the web console's [`LogRing`]. Bounded and
/// lossy by construction; `on_event` never blocks and never awaits.
struct RingLayer {
    ring: Arc<LogRing>,
}

impl<S: tracing::Subscriber> Layer<S> for RingLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let message = talon_secrets::redact::global().scrub_owned(visitor.0);
        self.ring.push(LogLine {
            ts: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            level: event.metadata().level().to_string(),
            target: event.metadata().target().to_string(),
            message,
        });
    }
}

/// Collects `message` plus any other fields as `key=value`.
#[derive(Default)]
struct FieldVisitor(String);

impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let rendered = format!("{value:?}");
            if self.0.is_empty() {
                self.0 = rendered;
            } else {
                self.0 = format!("{rendered} {}", self.0);
            }
        } else {
            use std::fmt::Write as _;
            let _ = write!(self.0, " {}={:?}", field.name(), value);
        }
    }
}

// ── Config (criterion 16) ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggingConfig {
    /// Write the JSON file sink (default true).
    pub file: bool,
    /// stderr format: `pretty` (default) or `json`. The file sink is always
    /// JSON lines.
    pub json_stderr: bool,
    /// Level when neither `RUST_LOG` nor `--log-level` overrides.
    pub level: Option<String>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            file: true,
            json_stderr: false,
            level: None,
        }
    }
}

impl LoggingConfig {
    /// Parse `[logging]` from a config.toml text; absent/garbled → defaults.
    pub fn from_config_text(text: &str) -> Self {
        let Ok(value) = text.parse::<toml::Value>() else {
            return Self::default();
        };
        let Some(logging) = value.get("logging") else {
            return Self::default();
        };
        Self {
            file: logging
                .get("file")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            json_stderr: logging
                .get("format")
                .and_then(|v| v.as_str())
                .map(|f| f.eq_ignore_ascii_case("json"))
                .unwrap_or(false),
            level: logging
                .get("level")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        }
    }

    fn load(talon_home: Option<&PathBuf>) -> Self {
        talon_home
            .map(|home| home.join("config.toml"))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|text| Self::from_config_text(&text))
            .unwrap_or_default()
    }
}

// ── Init ─────────────────────────────────────────────────────────────────────

/// Keeps the file-appender worker and the console ring alive for the process.
pub struct LogHandle {
    pub ring: Arc<LogRing>,
    _file_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

/// Build and install the global subscriber. Precedence for the filter:
/// `RUST_LOG` → `--log-level` (when not the default) → `[logging] level` →
/// `info`.
pub fn init(cli_level: &str, talon_home: Option<PathBuf>) -> anyhow::Result<LogHandle> {
    let cfg = LoggingConfig::load(talon_home.as_ref());

    let effective = if std::env::var("RUST_LOG").is_ok() {
        None // EnvFilter reads RUST_LOG itself via try_from_default_env
    } else if cli_level != "info" {
        Some(cli_level.to_string())
    } else {
        Some(cfg.level.clone().unwrap_or_else(|| "info".to_string()))
    };
    let filter = match effective {
        Some(lvl) => EnvFilter::try_new(&lvl).or_else(|_| EnvFilter::try_new("info"))?,
        None => EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new("info"))?,
    };

    // stderr — pretty default, json opt-in; always scrubbed.
    let stderr_layer = if cfg.json_stderr {
        tracing_subscriber::fmt::layer()
            .json()
            .with_writer(ScrubStderr)
            .boxed()
    } else {
        tracing_subscriber::fmt::layer()
            .with_target(false)
            .with_writer(ScrubStderr)
            .boxed()
    };

    // JSON file sink with daily rotation; non-blocking so a slow disk never
    // stalls the runtime.
    let (file_layer, file_guard) = if cfg.file && talon_home.is_some() {
        let dir = talon_home
            .as_ref()
            .map(|h| h.join("logs"))
            .unwrap_or_else(|| PathBuf::from("logs"));
        match std::fs::create_dir_all(&dir) {
            Ok(()) => {
                let appender = tracing_appender::rolling::daily(&dir, "talon.log");
                let (non_blocking, guard) = tracing_appender::non_blocking(appender);
                let layer = tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(ScrubMakeWriter(non_blocking))
                    .boxed();
                (Some(layer), Some(guard))
            }
            Err(e) => {
                eprintln!(
                    "warning: cannot create {} — file logging disabled: {e}",
                    dir.display()
                );
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    let ring = Arc::new(LogRing::new());
    let ring_layer = RingLayer {
        ring: Arc::clone(&ring),
    };

    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .with(file_layer)
        .with(ring_layer);

    #[cfg(feature = "otel")]
    let registry = registry.with(otel::layer(talon_home.as_ref()));

    registry.try_init()?;
    Ok(LogHandle {
        ring,
        _file_guard: file_guard,
    })
}

// ── OTLP export (criterion 18, feature = "otel") ─────────────────────────────

#[cfg(feature = "otel")]
mod otel {
    use std::path::PathBuf;

    /// Build the OTLP layer when `[otel] endpoint` is configured; `None`
    /// otherwise (no exporter, zero hot-path cost).
    pub fn layer<S>(talon_home: Option<&PathBuf>) -> Option<impl tracing_subscriber::Layer<S>>
    where
        S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    {
        let endpoint = endpoint_from_config(talon_home)?;

        use opentelemetry_otlp::WithExportConfig as _;
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(endpoint.clone())
            .build()
            .map_err(|e| eprintln!("warning: otel exporter init failed: {e}"))
            .ok()?;
        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .build();
        use opentelemetry::trace::TracerProvider as _;
        let tracer = provider.tracer("talon");
        eprintln!("otel: exporting spans to {endpoint}");
        Some(tracing_opentelemetry::layer().with_tracer(tracer))
    }

    fn endpoint_from_config(talon_home: Option<&PathBuf>) -> Option<String> {
        let text = std::fs::read_to_string(talon_home?.join("config.toml")).ok()?;
        let value: toml::Value = text.parse().ok()?;
        value
            .get("otel")?
            .get("endpoint")?
            .as_str()
            .map(str::to_string)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn scrub_writer_redacts_registered_values() {
        let _g = talon_secrets::redact::global().register("LOGKEY", "log-secret-value-91");
        let mut sink = ScrubWriter(Vec::new());
        sink.write_all(b"before log-secret-value-91 after\n")
            .expect("write");

        let written = String::from_utf8(sink.0).expect("utf8");
        assert!(!written.contains("log-secret-value-91"));
        assert!(written.contains("[REDACTED:LOGKEY]"));
    }

    #[test]
    fn scrub_writer_passes_clean_lines_through() {
        let mut sink = ScrubWriter(Vec::new());
        sink.write_all(b"nothing secret here\n").expect("write");
        assert_eq!(sink.0, b"nothing secret here\n");
    }

    #[test]
    fn logging_config_defaults() {
        let cfg = LoggingConfig::from_config_text("");
        assert_eq!(cfg, LoggingConfig::default());
        assert!(cfg.file, "file sink defaults on");
        assert!(!cfg.json_stderr, "stderr defaults pretty");
    }

    #[test]
    fn logging_config_parses_table() {
        let cfg = LoggingConfig::from_config_text(
            "[logging]\nfile = false\nformat = \"json\"\nlevel = \"debug\"\n",
        );
        assert!(!cfg.file);
        assert!(cfg.json_stderr);
        assert_eq!(cfg.level.as_deref(), Some("debug"));
    }

    #[test]
    fn ring_layer_captures_events_with_levels() {
        use tracing_subscriber::layer::SubscriberExt;

        let ring = Arc::new(LogRing::new());
        let subscriber = tracing_subscriber::registry().with(RingLayer {
            ring: Arc::clone(&ring),
        });

        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(job = "j1", "ring capture test");
        });

        let snap = ring.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].level, "WARN");
        assert!(snap[0].message.contains("ring capture test"));
        assert!(snap[0].message.contains("job=\"j1\""));
    }

    #[test]
    fn ring_layer_scrubs_secret_values() {
        use tracing_subscriber::layer::SubscriberExt;

        let _g = talon_secrets::redact::global().register("RINGKEY", "ring-secret-77");
        let ring = Arc::new(LogRing::new());
        let subscriber = tracing_subscriber::registry().with(RingLayer {
            ring: Arc::clone(&ring),
        });

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("leaking ring-secret-77 maybe");
        });

        let snap = ring.snapshot();
        assert!(!snap[0].message.contains("ring-secret-77"));
        assert!(snap[0].message.contains("[REDACTED:RINGKEY]"));
    }
}
