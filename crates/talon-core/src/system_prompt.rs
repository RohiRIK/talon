//! Baseline system prompt injected at the head of every conversation.
//!
//! This is Talon's equivalent of the "initial system message" pattern (Hermes'
//! SolMD injection): a small, stable block of instructions prepended to every
//! LLM call so the model always knows its identity and where its memory lives.
//!
//! Kept deliberately small and Markdown-first (stable content first, so the
//! provider prompt cache prefix stays intact). Future capabilities — retrieved
//! memories, user profile, tool guidance — are appended as additional sections
//! rather than rewritten, which is why this is a builder returning `String`.

/// The stable, cache-friendly baseline. Stays identical across turns so the
/// provider can cache this prefix. New durable sections go here; volatile,
/// per-turn context (retrieved memories, user input) is appended downstream.
const BASELINE: &str = "\
# Talon

You are Talon, a single-binary AI agent that runs locally.

## Memory

Your long-term memory is a local SQLite database (`~/.talon/talon.db`). It is
queryable across sessions and projects — no cloud, no external service. Facts,
their embeddings, and a full-text index all live in that one file. Hybrid
retrieval combines FTS5 keyword search with sqlite-vec vector search. When you
recall or store something, it persists there.";

/// Build the baseline system prompt. Currently returns the stable [`BASELINE`];
/// the signature is a function (not a `const`) so future callers can compose in
/// dynamic context (user profile, retrieved memories) without changing callers.
pub fn baseline_system_prompt() -> String {
    BASELINE.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_states_sqlite_memory() {
        let p = baseline_system_prompt();
        assert!(
            p.contains("SQLite"),
            "must tell Talon its memory is on SQLite"
        );
        assert!(p.contains("talon.db"));
    }

    #[test]
    fn baseline_identifies_talon() {
        assert!(baseline_system_prompt().contains("You are Talon"));
    }

    #[test]
    fn baseline_is_markdown_not_xml() {
        let p = baseline_system_prompt();
        assert!(
            p.contains("# Talon"),
            "Markdown-first per prompting standards"
        );
        assert!(!p.contains('<'), "no XML tags in prompts");
    }
}
