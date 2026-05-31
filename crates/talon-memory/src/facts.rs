//! `FactExtractor` — LLM-powered extraction of durable memories (task 2.5.5).
//!
//! At session end, the transcript is handed to an LLM which returns a JSON array
//! of facts worth remembering across future sessions. Each becomes a [`Memory`]
//! with a category, importance, tags, and entities.
//!
//! The LLM is injected via the [`FactCompleter`] trait so `talon-memory` stays
//! free of a `talon-llm` dependency (same pattern as [`crate::Summarizer`]).
//! The extraction prompt is Markdown-first with stable instructions first and
//! the volatile transcript last (prompt-cache friendly).

use std::{future::Future, pin::Pin};

use serde::Deserialize;

use crate::{Memory, MemoryCategory, MemoryError};

/// A single-shot text completion over a prompt. Implemented by an LLM-backed
/// type outside this crate; a deterministic stub is used in tests.
pub trait FactCompleter: Send + Sync {
    fn complete<'a>(
        &'a self,
        prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, MemoryError>> + Send + 'a>>;
}

/// Extracts durable memories from a conversation transcript.
#[derive(Debug, Default, Clone, Copy)]
pub struct FactExtractor;

impl FactExtractor {
    pub fn new() -> Self {
        Self
    }

    /// Build the extraction prompt for a transcript. Stable instructions first,
    /// transcript (volatile) last.
    pub fn extraction_prompt(transcript: &str) -> String {
        format!("{EXTRACTION_INSTRUCTIONS}\n\n## Transcript\n\n{transcript}\n")
    }

    /// Run extraction: prompt the LLM and parse its JSON response into memories.
    pub async fn extract(
        &self,
        transcript: &str,
        llm: &dyn FactCompleter,
    ) -> Result<Vec<Memory>, MemoryError> {
        let prompt = Self::extraction_prompt(transcript);
        let raw = llm.complete(&prompt).await?;
        Self::parse(&raw)
    }

    /// Parse an LLM response into memories. Tolerant of surrounding prose and
    /// Markdown code fences: the first `[ … ]` JSON array is used. Returns an
    /// empty vec when no array is present (the model found nothing to remember).
    pub fn parse(raw: &str) -> Result<Vec<Memory>, MemoryError> {
        let Some(json) = extract_json_array(raw) else {
            return Ok(Vec::new());
        };
        let facts: Vec<ExtractedFact> = serde_json::from_str(json)?;
        Ok(facts
            .into_iter()
            .filter(|f| !f.content.trim().is_empty())
            .map(ExtractedFact::into_memory)
            .collect())
    }
}

/// The stable, cache-friendly head of the extraction prompt.
const EXTRACTION_INSTRUCTIONS: &str = "\
# Fact Extraction

You extract durable, reusable memories from a conversation transcript. These are
remembered across future sessions, so capture only what stays true and useful.

## Categories

- `user_preference` — a stable preference (e.g. \"prefers dark mode\", \"uses bun not npm\")
- `decision` — a choice that was made and should not be relitigated
- `fact` — a durable fact about the user, project, or environment
- `pattern` — a convention or approach worth reusing
- `gotcha` — a pitfall or warning to avoid repeating

## Ignore

- Greetings, chit-chat, and one-off task state
- Anything trivial or easily re-derivable
- Secrets, credentials, or tokens — never store these

## Output

Return ONLY a JSON array, nothing else. Each element:

`{\"content\": \"<one concise sentence>\", \"category\": \"<category>\", \"importance\": <1-5>, \"tags\": [\"...\"], \"entities\": [\"...\"]}`

- `importance`: 5 = load-bearing and durable, 1 = barely worth keeping
- `tags`: short topical labels · `entities`: named people, files, or projects
- If there is nothing worth remembering, return `[]`.";

/// Locate the outermost JSON array in a possibly-noisy LLM response.
fn extract_json_array(raw: &str) -> Option<&str> {
    let start = raw.find('[')?;
    let end = raw.rfind(']')?;
    (end >= start).then(|| &raw[start..=end])
}

#[derive(Debug, Deserialize)]
struct ExtractedFact {
    content: String,
    #[serde(default)]
    category: MemoryCategory,
    #[serde(default = "default_importance")]
    importance: u8,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    entities: Vec<String>,
}

fn default_importance() -> u8 {
    3
}

impl ExtractedFact {
    fn into_memory(self) -> Memory {
        Memory::new(
            self.content,
            self.category,
            self.importance.clamp(1, 5),
            self.tags,
            self.entities,
        )
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    struct StubLlm(&'static str);
    impl FactCompleter for StubLlm {
        fn complete<'a>(
            &'a self,
            _prompt: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<String, MemoryError>> + Send + 'a>> {
            let out = self.0.to_string();
            Box::pin(async move { Ok(out) })
        }
    }

    #[test]
    fn prompt_includes_transcript_and_categories() {
        let p = FactExtractor::extraction_prompt("user: I use bun\nassistant: noted");
        assert!(p.contains("user_preference"));
        assert!(p.contains("I use bun"));
        // Transcript comes after the instructions (cache-friendly ordering).
        assert!(p.find("# Fact Extraction") < p.find("## Transcript"));
    }

    #[test]
    fn parse_plain_json_array() {
        let raw = r#"[{"content":"prefers dark mode","category":"user_preference","importance":4,"tags":["ui"],"entities":["dark mode"]}]"#;
        let mems = FactExtractor::parse(raw).expect("parse");
        assert_eq!(mems.len(), 1);
        assert_eq!(mems[0].content, "prefers dark mode");
        assert_eq!(mems[0].category, MemoryCategory::UserPreference);
        assert_eq!(mems[0].importance, 4);
        assert_eq!(mems[0].tags, vec!["ui".to_string()]);
        assert_eq!(mems[0].entities, vec!["dark mode".to_string()]);
    }

    #[test]
    fn parse_strips_surrounding_fences_and_prose() {
        let raw = "Here are the facts:\n```json\n[{\"content\":\"uses zed\",\"category\":\"user_preference\"}]\n```\nDone.";
        let mems = FactExtractor::parse(raw).expect("parse");
        assert_eq!(mems.len(), 1);
        assert_eq!(mems[0].content, "uses zed");
        // Missing importance defaults to 3.
        assert_eq!(mems[0].importance, 3);
    }

    #[test]
    fn parse_no_array_returns_empty() {
        let mems = FactExtractor::parse("No durable facts found.").expect("parse");
        assert!(mems.is_empty());
    }

    #[test]
    fn parse_empty_array_returns_empty() {
        assert!(FactExtractor::parse("[]").expect("parse").is_empty());
    }

    #[test]
    fn parse_clamps_out_of_range_importance() {
        let raw = r#"[{"content":"a","importance":9},{"content":"b","importance":0}]"#;
        let mems = FactExtractor::parse(raw).expect("parse");
        assert_eq!(mems[0].importance, 5);
        assert_eq!(mems[1].importance, 1);
    }

    #[test]
    fn parse_skips_blank_content() {
        let raw = r#"[{"content":"  "},{"content":"real"}]"#;
        let mems = FactExtractor::parse(raw).expect("parse");
        assert_eq!(mems.len(), 1);
        assert_eq!(mems[0].content, "real");
    }

    #[test]
    fn parse_unknown_category_defaults_to_fact() {
        let raw = r#"[{"content":"x","category":"fact"}]"#;
        let mems = FactExtractor::parse(raw).expect("parse");
        assert_eq!(mems[0].category, MemoryCategory::Fact);
    }

    #[tokio::test]
    async fn extract_prompts_llm_and_parses() {
        let llm =
            StubLlm(r#"[{"content":"prefers tabs","category":"user_preference","importance":2}]"#);
        let mems = FactExtractor::new()
            .extract("user: I like tabs", &llm)
            .await
            .expect("extract");
        assert_eq!(mems.len(), 1);
        assert_eq!(mems[0].content, "prefers tabs");
        assert_eq!(mems[0].importance, 2);
    }

    #[tokio::test]
    async fn extract_handles_empty_model_output() {
        let llm = StubLlm("nothing worth saving");
        let mems = FactExtractor::new()
            .extract("user: hi", &llm)
            .await
            .expect("extract");
        assert!(mems.is_empty());
    }
}
