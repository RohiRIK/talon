//! Integration tests for talon-memory.
//!
//! These tests run against a real in-memory SQLite database and verify:
//! - 100+ message workload round-trip
//! - FTS5 search latency <50ms
//! - ContextBuilder stays under token budget

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Instant;

use talon_memory::{ContextBuilder, Database, MemoryStore, SqliteStore};

async fn make_db() -> Arc<Database> {
    let db = Arc::new(Database::open(":memory:").expect("open"));
    db.init_schema().await.expect("schema");
    db
}

async fn make_store() -> Arc<SqliteStore> {
    Arc::new(SqliteStore::new(make_db().await))
}

/// Insert n messages into the given store under session "bench-session".
async fn populate(store: &dyn MemoryStore, n: usize) {
    for i in 0..n {
        let role = if i % 2 == 0 { "user" } else { "assistant" };
        let content = format!(
            "Message {i}: Talon is a memory-first AI coding assistant built in Rust."
        );
        store.save_message("bench-session", role, &content).await.expect("save");
    }
}

// ── 100+ message round-trip ───────────────────────────────────────────────────

#[tokio::test]
async fn insert_100_messages_and_retrieve_all() {
    let store = make_store().await;
    populate(store.as_ref(), 100).await;

    let rows = store.recent_messages("bench-session", 100).await.expect("recent");
    assert_eq!(rows.len(), 100, "expected 100 messages, got {}", rows.len());
    // Oldest-first order: first message should mention index 0.
    assert!(rows[0].content.contains("Message 0"), "first row: {}", rows[0].content);
    assert!(rows[99].content.contains("Message 99"), "last row: {}", rows[99].content);
}

#[tokio::test]
async fn insert_200_messages_recent_window_respects_limit() {
    let store = make_store().await;
    populate(store.as_ref(), 200).await;

    let rows = store.recent_messages("bench-session", 50).await.expect("recent");
    assert_eq!(rows.len(), 50, "expected 50 most recent, got {}", rows.len());
    // The 50 most recent should be messages 150–199.
    assert!(rows[0].content.contains("Message 150"), "oldest of window: {}", rows[0].content);
    assert!(rows[49].content.contains("Message 199"), "newest of window: {}", rows[49].content);
}

// ── FTS5 search latency ───────────────────────────────────────────────────────

#[tokio::test]
async fn fts5_search_100_messages_under_50ms() {
    let store = make_store().await;
    populate(store.as_ref(), 100).await;

    let start = Instant::now();
    let results = store.search_messages("Rust", 10).await.expect("search");
    let elapsed = start.elapsed();

    assert!(!results.is_empty(), "expected matches for 'Rust'");
    assert!(
        elapsed.as_millis() < 50,
        "FTS5 search took {}ms — must be <50ms",
        elapsed.as_millis()
    );
}

#[tokio::test]
async fn fts5_search_finds_needle_in_100_messages() {
    let store = make_store().await;
    populate(store.as_ref(), 99).await;
    // Insert one unique message that stands out.
    store
        .save_message("bench-session", "user", "unique_needle_xq9 is here")
        .await
        .expect("save");

    let results = store.search_messages("unique_needle_xq9", 5).await.expect("search");
    assert_eq!(results.len(), 1, "should find exactly the needle");
    assert!(results[0].content.contains("unique_needle_xq9"));
}

// ── ContextBuilder token budget ───────────────────────────────────────────────

#[tokio::test]
async fn context_builder_stays_under_token_budget() {
    let db = make_db().await;
    let store: Arc<dyn MemoryStore> = Arc::new(SqliteStore::new(Arc::clone(&db)));

    // Insert 100 messages (~40 chars each → ~10 tokens each → 1000 tokens total).
    for i in 0..100 {
        store
            .save_message("ctx-session", "user", &format!("message content number {i} here"))
            .await
            .expect("save");
    }

    let budget = 200;
    let ctx = ContextBuilder::new(store.as_ref(), "ctx-session")
        .system_prompt("You are Talon.")
        .max_tokens(budget)
        .recent_n(100)
        .build()
        .await
        .expect("build");

    assert!(
        ctx.estimated_tokens <= budget,
        "context used {} tokens, budget was {budget}",
        ctx.estimated_tokens
    );
}

#[tokio::test]
async fn context_builder_includes_fts_hits_from_other_sessions() {
    let db = make_db().await;
    let store: Arc<dyn MemoryStore> = Arc::new(SqliteStore::new(Arc::clone(&db)));

    // Old session with relevant content.
    store
        .save_message("old-session", "user", "we discussed Rust ownership rules")
        .await
        .expect("save");

    // Current session has nothing yet.
    let ctx = ContextBuilder::new(store.as_ref(), "new-session")
        .fts_query("ownership")
        .fts_limit(3)
        .build()
        .await
        .expect("build");

    let found = ctx.messages.iter().any(|m| m.content.contains("ownership"));
    assert!(found, "FTS hit from old-session should appear in context");
}

// ── Stats and vacuum ──────────────────────────────────────────────────────────

#[tokio::test]
async fn stats_reflect_inserted_messages() {
    let db = make_db().await;
    let store = SqliteStore::new(Arc::clone(&db));
    populate(&store, 50).await;

    let s = db.stats().await.expect("stats");
    assert_eq!(s.session_count, 1, "expected 1 session");
    assert_eq!(s.message_count, 50, "expected 50 messages");
    assert!(s.size_bytes > 0, "size_bytes should be >0");
}

#[tokio::test]
async fn vacuum_after_deletes_does_not_error() {
    let db = make_db().await;
    let store = SqliteStore::new(Arc::clone(&db));
    populate(&store, 20).await;

    db.pool()
        .get()
        .await
        .expect("pool")
        .interact(|conn| conn.execute_batch("DELETE FROM messages WHERE 1=1"))
        .await
        .expect("interact")
        .expect("delete");

    db.vacuum().await.expect("vacuum after delete");
    let s = db.stats().await.expect("stats");
    assert_eq!(s.message_count, 0);
}
