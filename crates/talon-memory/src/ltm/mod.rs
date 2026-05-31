//! Talon LTM — long-term memory model and SQLite-backed store.
//!
//! ADR 0008: memories live in the same `talon.db` as sessions and messages.
//! Each memory carries its FTS5 index row and (optionally) a `sqlite-vec`
//! embedding, all written in one transaction.
//!
//! The typed `MemoryCategory` enum, `tags`, and richer constructors arrive in
//! task 2.5.3; this module currently models `category` as a plain string so the
//! storage layer (2.5.2) can land and be exercised independently.

pub mod store;

pub use store::LtmStore;

/// A single long-term memory as stored in the `memories` table.
#[derive(Debug, Clone, PartialEq)]
pub struct Memory {
    pub id: i64,
    pub content: String,
    pub category: String,
    /// 1–5; enforced by a CHECK constraint in the schema.
    pub importance: u8,
    pub created_at: i64,
    pub accessed_at: i64,
    pub decay_score: f32,
    pub entities: Vec<String>,
}

impl Memory {
    /// Build a memory for insertion. `id`, timestamps, and `decay_score` are
    /// assigned by the database — they stay zeroed here and are populated on read.
    pub fn new(
        content: impl Into<String>,
        category: impl Into<String>,
        importance: u8,
        entities: Vec<String>,
    ) -> Self {
        Self {
            id: 0,
            content: content.into(),
            category: category.into(),
            importance,
            created_at: 0,
            accessed_at: 0,
            decay_score: 0.0,
            entities,
        }
    }
}
