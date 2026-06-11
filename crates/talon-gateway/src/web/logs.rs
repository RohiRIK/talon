//! Live log tail — bounded ring buffer + `GET /api/v1/logs/tail` SSE
//! (criterion 20).
//!
//! [`LogRing`] is a plain data structure: the binary's tracing layer pushes
//! [`LogLine`]s in; this handler streams the buffered backlog followed by
//! live lines. Bounded and lossy (drop-oldest) — a slow console tab must
//! never apply backpressure to the tracing pipeline. Lines arrive already
//! scrubbed (the redaction layer sits upstream in the subscriber stack).

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Mutex;

use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::Stream;
use tokio::sync::broadcast;

use super::WebState;

/// Buffered backlog size; also the live-channel capacity.
pub const LOG_RING_CAP: usize = 500;

/// One formatted log event.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LogLine {
    /// UTC RFC3339.
    pub ts: String,
    /// `ERROR | WARN | INFO | DEBUG | TRACE`.
    pub level: String,
    pub target: String,
    pub message: String,
}

impl LogLine {
    /// Numeric severity for filtering: higher = more severe.
    fn severity(&self) -> u8 {
        match self.level.as_str() {
            "ERROR" => 5,
            "WARN" => 4,
            "INFO" => 3,
            "DEBUG" => 2,
            _ => 1,
        }
    }
}

fn severity_of(level: &str) -> u8 {
    match level.to_ascii_uppercase().as_str() {
        "ERROR" => 5,
        "WARN" => 4,
        "INFO" => 3,
        "DEBUG" => 2,
        _ => 1,
    }
}

/// Bounded drop-oldest buffer + live broadcast.
pub struct LogRing {
    buf: Mutex<VecDeque<LogLine>>,
    tx: broadcast::Sender<LogLine>,
}

impl Default for LogRing {
    fn default() -> Self {
        Self::new()
    }
}

impl LogRing {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(LOG_RING_CAP);
        Self {
            buf: Mutex::new(VecDeque::with_capacity(LOG_RING_CAP)),
            tx,
        }
    }

    /// Push a line: never blocks, never errors, drops the oldest at capacity.
    pub fn push(&self, line: LogLine) {
        {
            let mut buf = self.buf.lock().unwrap_or_else(|e| e.into_inner());
            if buf.len() == LOG_RING_CAP {
                buf.pop_front();
            }
            buf.push_back(line.clone());
        }
        let _ = self.tx.send(line);
    }

    pub fn snapshot(&self) -> Vec<LogLine> {
        self.buf
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LogLine> {
        self.tx.subscribe()
    }
}

#[derive(serde::Deserialize)]
pub struct TailParams {
    /// Minimum level (`error|warn|info|debug|trace`); default `trace` = all.
    pub level: Option<String>,
}

/// `GET /api/v1/logs/tail?level=` — backlog snapshot, then live lines.
pub async fn tail(
    State(state): State<WebState>,
    Query(params): Query<TailParams>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let min = params.level.as_deref().map(severity_of).unwrap_or(1);

    let (backlog, rx) = match &state.log_ring {
        Some(ring) => (ring.snapshot(), Some(ring.subscribe())),
        None => (vec![], None),
    };

    let stream = async_stream(backlog, rx, min);
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn async_stream(
    backlog: Vec<LogLine>,
    rx: Option<broadcast::Receiver<LogLine>>,
    min: u8,
) -> impl Stream<Item = Result<Event, Infallible>> {
    futures::stream::unfold(
        (backlog.into_iter(), rx),
        move |(mut backlog, mut rx)| async move {
            // Drain the snapshot first, then follow the live channel.
            loop {
                if let Some(line) = backlog.next() {
                    if line.severity() < min {
                        continue;
                    }
                    let event = to_event(&line);
                    return Some((Ok(event), (backlog, rx)));
                }
                let live = rx.as_mut()?;
                match live.recv().await {
                    Ok(line) if line.severity() >= min => {
                        let event = to_event(&line);
                        return Some((Ok(event), (backlog, rx)));
                    }
                    Ok(_) => continue,
                    // Lagged: skip the gap marker, keep tailing.
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    )
}

fn to_event(line: &LogLine) -> Event {
    let data = serde_json::to_string(line).unwrap_or_else(|_| "{}".to_string());
    // Belt-and-braces: lines are scrubbed upstream, but this sink obeys the
    // same rule as the run-event SSE feed.
    let data = talon_secrets::redact::global().scrub_owned(data);
    Event::default().data(data)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn line(level: &str, msg: &str) -> LogLine {
        LogLine {
            ts: "2026-06-11T00:00:00Z".into(),
            level: level.into(),
            target: "test".into(),
            message: msg.into(),
        }
    }

    #[test]
    fn ring_drops_oldest_at_capacity() {
        let ring = LogRing::new();
        for i in 0..(LOG_RING_CAP + 10) {
            ring.push(line("INFO", &format!("m{i}")));
        }
        let snap = ring.snapshot();
        assert_eq!(snap.len(), LOG_RING_CAP);
        assert_eq!(snap[0].message, "m10", "oldest dropped");
    }

    #[tokio::test]
    async fn subscriber_sees_live_lines() {
        let ring = LogRing::new();
        let mut rx = ring.subscribe();
        ring.push(line("WARN", "live"));
        let got = rx.recv().await.expect("line");
        assert_eq!(got.message, "live");
    }

    #[test]
    fn severity_ordering() {
        assert!(line("ERROR", "").severity() > line("WARN", "").severity());
        assert!(line("WARN", "").severity() > line("INFO", "").severity());
        assert_eq!(severity_of("warn"), 4, "case-insensitive");
    }
}
