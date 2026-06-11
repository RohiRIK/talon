//! Tracing output scrubbing (spec criterion 10).
//!
//! Wraps the subscriber's writer so every formatted log line passes through
//! the global redaction registry before reaching the terminal. While a job
//! run holds redaction guards, any log line containing a resolved secret
//! value is rewritten to `[REDACTED:<name>]`.

use std::io::{self, Write};

use tracing_subscriber::fmt::MakeWriter;

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
}
