//! Process-wide redaction registry (criterion 10).
//!
//! Resolved secret values are registered for the lifetime of a run (RAII
//! guard) and scrubbed — at choke points, not call sites — from everything
//! that leaves the process: run records, tracing output, SSE payloads.
//!
//! Scrub is plain multi-pattern substring replacement. Values shorter than
//! [`MIN_VALUE_LEN`] are refused (registering "a" would shred unrelated text).
//! The empty-registry path is a single `RwLock` read + `None` check.

use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};

use aho_corasick::AhoCorasick;

/// Values shorter than this are never registered for redaction.
pub const MIN_VALUE_LEN: usize = 4;

static GLOBAL: OnceLock<RedactionRegistry> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// The process-global registry — the one every sink consults.
pub fn global() -> &'static RedactionRegistry {
    GLOBAL.get_or_init(RedactionRegistry::new)
}

/// Scrub `text` against the global registry. The common no-secrets case
/// borrows and allocates nothing.
pub fn scrub(text: &str) -> Cow<'_, str> {
    global().scrub(text)
}

struct Entry {
    id: u64,
    name: String,
    value: String,
}

#[derive(Default)]
struct Inner {
    entries: Vec<Entry>,
    /// Rebuilt on every register/unregister; `None` when no entries.
    automaton: Option<AhoCorasick>,
}

impl Inner {
    fn rebuild(&mut self) {
        self.automaton = if self.entries.is_empty() {
            None
        } else {
            AhoCorasick::new(self.entries.iter().map(|e| e.value.as_bytes())).ok()
        };
    }
}

pub struct RedactionRegistry {
    inner: RwLock<Inner>,
}

impl RedactionRegistry {
    fn new() -> Self {
        Self {
            inner: RwLock::new(Inner::default()),
        }
    }

    /// Register a value under `name`; returns a guard that unregisters on
    /// drop (run lifetime). Too-short values return a no-op guard.
    pub fn register(&'static self, name: &str, value: &str) -> RedactionGuard {
        if value.len() < MIN_VALUE_LEN {
            return RedactionGuard {
                registry: self,
                id: 0,
            };
        }
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.entries.push(Entry {
            id,
            name: name.to_string(),
            value: value.to_string(),
        });
        inner.rebuild();
        RedactionGuard { registry: self, id }
    }

    fn unregister(&self, id: u64) {
        if id == 0 {
            return;
        }
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.entries.retain(|e| e.id != id);
        inner.rebuild();
    }

    /// Replace every registered value with `[REDACTED:<name>]`.
    pub fn scrub<'t>(&self, text: &'t str) -> Cow<'t, str> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let Some(ac) = &inner.automaton else {
            return Cow::Borrowed(text);
        };
        if ac.find(text).is_none() {
            return Cow::Borrowed(text);
        }
        let replacements: Vec<String> = inner
            .entries
            .iter()
            .map(|e| format!("[REDACTED:{}]", e.name))
            .collect();
        Cow::Owned(ac.replace_all(text, &replacements))
    }

    /// Scrub an owned string in place (avoids a copy when nothing matches).
    pub fn scrub_owned(&self, text: String) -> String {
        match self.scrub(&text) {
            Cow::Borrowed(_) => text,
            Cow::Owned(s) => s,
        }
    }

    /// Convenience for `Option<String>` sink fields.
    pub fn scrub_opt(&self, text: Option<String>) -> Option<String> {
        text.map(|t| self.scrub_owned(t))
    }
}

/// RAII handle: dropping it removes the value from the registry.
pub struct RedactionGuard {
    registry: &'static RedactionRegistry,
    id: u64,
}

impl Drop for RedactionGuard {
    fn drop(&mut self) {
        self.registry.unregister(self.id);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    // NOTE: tests share the process-global registry; each test uses unique
    // values so concurrent tests cannot interfere.

    #[test]
    fn scrub_replaces_all_occurrences_including_embedded() {
        let g = global().register("API_KEY", "sk-abc-123-xyz");
        let text = r#"plain sk-abc-123-xyz and json {"k":"sk-abc-123-xyz"}"#;
        let scrubbed = scrub(text);
        assert!(!scrubbed.contains("sk-abc-123-xyz"));
        assert_eq!(scrubbed.matches("[REDACTED:API_KEY]").count(), 2);
        drop(g);
    }

    #[test]
    fn guard_drop_unregisters() {
        let value = "guard-test-value-unique-77";
        {
            let _g = global().register("G", value);
            assert!(scrub(value).contains("[REDACTED:G]"));
        }
        assert_eq!(scrub(value), value, "after drop, value passes through");
    }

    #[test]
    fn empty_registry_borrows() {
        let text = "no secrets registered in this exact string 9912";
        assert!(matches!(scrub(text), Cow::Borrowed(_)));
    }

    #[test]
    fn short_values_are_never_registered() {
        let _g = global().register("TINY", "ab");
        assert_eq!(scrub("ab cd ab"), "ab cd ab");
    }

    #[test]
    fn multiple_values_each_get_their_name() {
        let _g1 = global().register("ONE", "value-one-unique-31");
        let _g2 = global().register("TWO", "value-two-unique-32");
        let scrubbed = scrub("a value-one-unique-31 b value-two-unique-32");
        assert!(scrubbed.contains("[REDACTED:ONE]"));
        assert!(scrubbed.contains("[REDACTED:TWO]"));
    }

    #[test]
    fn scrub_opt_handles_none_and_some() {
        let _g = global().register("OPT", "opt-value-unique-55");
        assert_eq!(global().scrub_opt(None), None);
        assert_eq!(
            global().scrub_opt(Some("x opt-value-unique-55".into())),
            Some("x [REDACTED:OPT]".to_string())
        );
    }
}
