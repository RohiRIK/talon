//! Talon LTM — long-term memory model and SQLite-backed store.
//!
//! ADR 0008: memories live in the same `talon.db` as sessions and messages.
//! Each memory carries its FTS5 index row and (optionally) a `sqlite-vec`
//! embedding, all written in one transaction.

use serde::{Deserialize, Serialize};

pub mod store;

pub use store::LtmStore;

/// What kind of thing a memory records. Serialized to the DB and to/from LLM
/// JSON as snake_case labels (`user_preference`, `decision`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    /// A stable preference of the user (e.g. "prefers dark mode").
    UserPreference,
    /// A decision that was made and should not be relitigated.
    Decision,
    /// A general fact about the user, project, or world.
    #[default]
    Fact,
    /// A recurring pattern or convention worth reusing.
    Pattern,
    /// A pitfall or warning to avoid repeating.
    Gotcha,
}

impl MemoryCategory {
    /// The canonical snake_case label stored in the `memories.category` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UserPreference => "user_preference",
            Self::Decision => "decision",
            Self::Fact => "fact",
            Self::Pattern => "pattern",
            Self::Gotcha => "gotcha",
        }
    }

    /// Parse a stored label back into a category. Unknown labels fall back to
    /// `Fact` so a hand-edited or future-versioned DB never fails a read.
    pub fn from_label(s: &str) -> Self {
        match s {
            "user_preference" => Self::UserPreference,
            "decision" => Self::Decision,
            "pattern" => Self::Pattern,
            "gotcha" => Self::Gotcha,
            _ => Self::Fact,
        }
    }
}

/// A single long-term memory as stored in the `memories` table.
#[derive(Debug, Clone, PartialEq)]
pub struct Memory {
    pub id: i64,
    pub content: String,
    pub category: MemoryCategory,
    /// 1–5; enforced by a CHECK constraint in the schema.
    pub importance: u8,
    pub created_at: i64,
    pub accessed_at: i64,
    pub decay_score: f32,
    /// Free-form labels for grouping/filtering.
    pub tags: Vec<String>,
    /// Named entities referenced by the memory (people, files, projects).
    pub entities: Vec<String>,
}

impl Memory {
    /// Build a memory for insertion. `id`, timestamps, and `decay_score` are
    /// assigned by the database — they stay zeroed here and are populated on read.
    pub fn new(
        content: impl Into<String>,
        category: MemoryCategory,
        importance: u8,
        tags: Vec<String>,
        entities: Vec<String>,
    ) -> Self {
        Self {
            id: 0,
            content: content.into(),
            category,
            importance,
            created_at: 0,
            accessed_at: 0,
            decay_score: 0.0,
            tags,
            entities,
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn category_label_roundtrips() {
        for c in [
            MemoryCategory::UserPreference,
            MemoryCategory::Decision,
            MemoryCategory::Fact,
            MemoryCategory::Pattern,
            MemoryCategory::Gotcha,
        ] {
            assert_eq!(MemoryCategory::from_label(c.as_str()), c);
        }
    }

    #[test]
    fn unknown_category_label_falls_back_to_fact() {
        assert_eq!(MemoryCategory::from_label("nonsense"), MemoryCategory::Fact);
    }

    #[test]
    fn category_default_is_fact() {
        assert_eq!(MemoryCategory::default(), MemoryCategory::Fact);
    }

    #[test]
    fn category_serializes_as_snake_case() {
        let json = serde_json::to_string(&MemoryCategory::UserPreference).expect("json");
        assert_eq!(json, "\"user_preference\"");
    }

    #[test]
    fn new_zeroes_db_assigned_fields() {
        let m = Memory::new("x", MemoryCategory::Decision, 5, vec![], vec![]);
        assert_eq!(m.id, 0);
        assert_eq!(m.created_at, 0);
        assert_eq!(m.category, MemoryCategory::Decision);
    }
}
