//! Integration tests for talon-memory.
//!
//! These tests run against a real in-memory SQLite database and verify:
//! - 100+ message workload round-trip
//! - FTS5 search latency <50ms
//! - ContextBuilder stays under token budget

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use talon_memory::{
    ContextBuilder, Database, DecayEngine, DedupOutcome, Deduplicator, FactCompleter,
    FactExtractor, HybridSearch, LtmStore, Memory, MemoryCategory, MemoryError, MemoryStore,
    SemanticCache, SqliteStore,
};

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
        let content =
            format!("Message {i}: Talon is a memory-first AI coding assistant built in Rust.");
        store
            .save_message("bench-session", role, &content)
            .await
            .expect("save");
    }
}

// ── 100+ message round-trip ───────────────────────────────────────────────────

#[tokio::test]
async fn insert_100_messages_and_retrieve_all() {
    let store = make_store().await;
    populate(store.as_ref(), 100).await;

    let rows = store
        .recent_messages("bench-session", 100)
        .await
        .expect("recent");
    assert_eq!(rows.len(), 100, "expected 100 messages, got {}", rows.len());
    // Oldest-first order: first message should mention index 0.
    assert!(
        rows[0].content.contains("Message 0"),
        "first row: {}",
        rows[0].content
    );
    assert!(
        rows[99].content.contains("Message 99"),
        "last row: {}",
        rows[99].content
    );
}

#[tokio::test]
async fn insert_200_messages_recent_window_respects_limit() {
    let store = make_store().await;
    populate(store.as_ref(), 200).await;

    let rows = store
        .recent_messages("bench-session", 50)
        .await
        .expect("recent");
    assert_eq!(
        rows.len(),
        50,
        "expected 50 most recent, got {}",
        rows.len()
    );
    // The 50 most recent should be messages 150–199.
    assert!(
        rows[0].content.contains("Message 150"),
        "oldest of window: {}",
        rows[0].content
    );
    assert!(
        rows[49].content.contains("Message 199"),
        "newest of window: {}",
        rows[49].content
    );
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

    let results = store
        .search_messages("unique_needle_xq9", 5)
        .await
        .expect("search");
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
            .save_message(
                "ctx-session",
                "user",
                &format!("message content number {i} here"),
            )
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

// ── Phase 2.5 LTM end-to-end (task 2.5.12) ────────────────────────────────────

async fn make_ltm() -> LtmStore {
    LtmStore::new(make_ltm_db().await)
}

async fn make_ltm_db() -> Database {
    let db = Database::open(":memory:").expect("open");
    db.init_schema().await.expect("schema");
    db
}

/// 384-dim embedding with two distinguishing leading components.
fn emb(a: f32, b: f32) -> Vec<f32> {
    let mut v = vec![0.0f32; 384];
    v[0] = a;
    v[1] = b;
    v
}

/// LLM stub that returns a fixed completion (used to drive fact extraction).
struct StubCompleter(&'static str);
impl FactCompleter for StubCompleter {
    fn complete<'a>(
        &'a self,
        _prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, MemoryError>> + Send + 'a>> {
        let out = self.0.to_string();
        Box::pin(async move { Ok(out) })
    }
}

#[tokio::test]
async fn fact_extraction_round_trips_into_ltm() {
    let store = make_ltm().await;
    let llm = StubCompleter(
        r#"Sure: [{"content":"user prefers dark mode","category":"user_preference","importance":4,"tags":["ui"],"entities":["dark mode"]}]"#,
    );

    let facts = FactExtractor::new()
        .extract("user: I always use dark mode", &llm)
        .await
        .expect("extract");
    assert_eq!(facts.len(), 1);

    let id = store.insert(&facts[0], None).await.expect("insert");
    let got = store.get(id).await.expect("get").expect("present");
    assert_eq!(got.content, "user prefers dark mode");
    assert_eq!(got.category, MemoryCategory::UserPreference);
    assert_eq!(got.importance, 4);

    // The extracted fact is keyword-recoverable in a later session.
    let hits = store.search_text("dark mode", 5).await.expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, id);
}

#[tokio::test]
async fn dedup_merges_near_duplicate_facts() {
    let store = make_ltm().await;
    let dedup = Deduplicator::new();

    let first = Memory::new(
        "user prefers dark mode",
        MemoryCategory::Fact,
        3,
        vec![],
        vec![],
    );
    let out = dedup
        .upsert(&store, &first, &emb(1.0, 0.0))
        .await
        .expect("upsert first");
    let id = match out {
        DedupOutcome::Inserted(id) => id,
        DedupOutcome::Merged { .. } => panic!("first insert must not merge"),
    };

    // A semantically identical fact with a near-identical vector must merge.
    let dup = Memory::new(
        "the user likes dark mode",
        MemoryCategory::Fact,
        5,
        vec![],
        vec![],
    );
    let out = dedup
        .upsert(&store, &dup, &emb(1.0, 0.0))
        .await
        .expect("upsert dup");
    match out {
        DedupOutcome::Merged {
            id: merged,
            similarity,
        } => {
            assert_eq!(merged, id, "must reinforce the existing memory");
            assert!(
                similarity >= 0.85,
                "similarity {similarity} below threshold"
            );
        }
        DedupOutcome::Inserted(_) => panic!("near-duplicate must merge, not insert"),
    }

    // Reinforcement raised importance to the higher of the two (5).
    assert_eq!(
        store
            .get(id)
            .await
            .expect("get")
            .expect("present")
            .importance,
        5
    );
}

#[tokio::test]
async fn hybrid_search_ranks_both_arm_match_first() {
    let store = make_ltm().await;
    let target = store
        .insert(
            &Memory::new(
                "user prefers dark mode",
                MemoryCategory::Fact,
                4,
                vec![],
                vec![],
            ),
            Some(&emb(1.0, 0.0)),
        )
        .await
        .expect("insert target");
    // Keyword-only match, far in vector space.
    store
        .insert(
            &Memory::new(
                "dark chocolate recipe",
                MemoryCategory::Fact,
                3,
                vec![],
                vec![],
            ),
            Some(&emb(0.0, 1.0)),
        )
        .await
        .expect("insert distractor");

    let hits = HybridSearch::new()
        .search(&store, "dark mode", &emb(1.0, 0.0), 5)
        .await
        .expect("search");
    assert_eq!(hits[0].id, target, "match strong in both arms ranks first");
}

#[tokio::test]
async fn semantic_cache_hits_and_misses() {
    let mut cache = SemanticCache::new(8, Duration::from_secs(60));
    cache.put(&emb(1.0, 0.0), "cached answer");

    // Identical embedding → hit.
    assert_eq!(cache.get(&emb(1.0, 0.0)).as_deref(), Some("cached answer"));
    // Orthogonal embedding → miss (cosine 0 < 0.95).
    assert!(cache.get(&emb(0.0, 1.0)).is_none());

    let stats = cache.stats();
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.entries, 1);
}

#[tokio::test]
async fn decay_reduces_score_over_time() {
    let store = make_ltm().await;
    let id = store
        .insert(
            &Memory::new("aging fact", MemoryCategory::Fact, 3, vec![], vec![]),
            None,
        )
        .await
        .expect("insert");

    let accessed = store
        .get(id)
        .await
        .expect("get")
        .expect("present")
        .accessed_at;
    let fresh = store
        .get(id)
        .await
        .expect("get")
        .expect("present")
        .decay_score;
    assert!(
        (fresh - 1.0).abs() < 1e-6,
        "new memory starts at full score"
    );

    // One half-life (30 days) after last access → score halves.
    let one_half_life = accessed + 30 * 86_400;
    let updated = DecayEngine::new()
        .run(&store, one_half_life)
        .await
        .expect("run");
    assert_eq!(updated, 1);

    let decayed = store
        .get(id)
        .await
        .expect("get")
        .expect("present")
        .decay_score;
    assert!(
        decayed < fresh && (decayed - 0.5).abs() < 1e-3,
        "score should erode to ~0.5, got {decayed}"
    );
}
