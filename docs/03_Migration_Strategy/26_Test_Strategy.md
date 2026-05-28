# Test Strategy & Coverage Targets

> **Last corrected:** dogfood pass 4

> **Status:** ✅ Complete
> **Category:** Migration Strategy

---

## 1. Test Pyramid

```
        ┌──────────┐
        │   E2E    │  ~5 tests — full agent turn, real LLM
        │ (CI-only)│  slow, expensive, external deps
        └────┬─────┘
       ┌─────┴──────┐
       │ Integration │  ~80 tests — DB ops, tools with mocked I/O
       │             │  fast, no network, tokio::test
       └──────┬──────┘
      ┌───────┴────────┐
      │     Unit        │  ~200 tests — pure logic, type conversions
      │                 │  instant, no async, no I/O
      └─────────────────┘
```

---

## 2. Coverage Targets

| Crate | Target | What's Tested |
|-------|--------|---------------|
| `talon-core` | 80% | Loop logic, [state machine](../02_Architecture/14_State_Machine_And_Lifecycle.md), context builder, [approval membrane](../02_Architecture/17a_Approval_Membrane.md) |
| `talon-llm` | 75% | [SSE parser](../05_API_Bindings/44_Streaming_SSE_Parser.md), Delta conversion, error mapping |
| `talon-memory` | 85% | SQL queries, FTS5, migration, skill parsing |
| `talon-tools` | 70% | Parameter validation, output formatting, error paths |
| `talon-gateway` | 60% | Message routing, rate limiting, media formatting |
| `talon-plugins` | 50% | WASM host, ABI boundary |

Measured with: `cargo llvm-cov --workspace`

---

## 3. Unit Tests — Pure Logic

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_human_interval_parsing() {
        assert_eq!(parse_human_interval("30m").unwrap(), "*/30 * * * *");
        assert_eq!(parse_human_interval("every 2h").unwrap(), "0 */2 * * *");
        assert_eq!(parse_human_interval("daily").unwrap(), "0 9 * * *");
        assert!(parse_human_interval("whenever").is_err());
    }

    #[test]
    fn test_approval_decision_readonly_always_allowed() {
        let membrane = ApprovalMembrane {
            default_level: ApprovalLevel::Dangerous,
            ..Default::default()
        };
        // ReadOnly tools are always approved regardless of level
        let call = ToolCall { name: "web_search".into(), ..Default::default() };
        // synchronous check for pure logic
        assert!(matches!(
            membrane.classify(&call, ToolRisk::ReadOnly),
            ApprovalDecision::Approved
        ));
    }

    #[test]
    fn test_tool_call_from_openai_wire_format() {
        let raw = serde_json::json!({
            "id": "call_abc",
            "type": "function",
            "function": {
                "name": "terminal",
                "arguments": "{\"command\":\"ls -la\"}"
            }
        });
        let call = ToolCall::from_openai_value(&raw).unwrap();
        assert_eq!(call.name, "terminal");
        assert_eq!(call.arguments["command"], "ls -la");
    }

    #[test]
    fn test_context_budget_allocation() {
        let budget = ContextBudget::for_model("claude-3-5-sonnet");
        assert_eq!(budget.total_tokens, 200_000);
        assert!(budget.system_tokens < budget.history_tokens);
        assert!(budget.system_tokens + budget.history_tokens
              + budget.tool_output_tokens + budget.reserve <= budget.total_tokens);
    }
}
```

---

## 4. Integration Tests — tokio::test + SQLite in-memory

```rust
#[cfg(test)]
mod integration {
    use super::*;

    async fn test_db() -> MemoryStore {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        apply_migrations(&conn).unwrap();
        MemoryStore::from_conn(conn)
    }

    #[tokio::test]
    async fn test_insert_and_fts5_search() {
        let store = test_db().await;
        let sid = Uuid::new_v4();

        store.insert_session(sid, "test", "cli").await.unwrap();
        store.insert_message(sid, "user", "Deploy the Tokio runtime".to_string()).await.unwrap();
        store.insert_message(sid, "assistant", "Sure, let me explain Tokio".to_string()).await.unwrap();

        let hits = store.search_messages("tokio runtime", 10).await.unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().any(|h| h.session_id == sid));
    }

    #[tokio::test]
    async fn test_skill_frontmatter_parsing() {
        let md = r#"---
name: github-pr-workflow
description: GitHub PR lifecycle management
category: github
pinned: false
---
# GitHub PR Workflow
..."#;
        let skill = SkillStore::parse_skill_md(md).unwrap();
        assert_eq!(skill.name, "github-pr-workflow");
        assert_eq!(skill.category.as_deref(), Some("github"));
        assert!(!skill.pinned);
    }

    #[tokio::test]
    async fn test_cron_job_upsert_and_list() {
        let store = test_db().await;
        let job = CronJob {
            id: Uuid::new_v4(),
            name: Some("test job".into()),
            schedule: CronSchedule::Human("30m".into()),
            prompt: "Say hello".into(),
            enabled: true,
            ..Default::default()
        };

        store.upsert_cron_job(&job).await.unwrap();
        let jobs = store.list_enabled_cron_jobs().await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name.as_deref(), Some("test job"));
    }
}
```

---

## 5. Contract Tests — LLM Provider Shapes

Use `httpmock` to record and replay real API responses:

```rust
#[tokio::test]
async fn test_openai_streaming_parse() {
    let server = httpmock::MockServer::start_async().await;

    server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(200)
            .body("data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: [DONE]\n\n");
    });

    let client = OpenAiCompatClient::new(
        server.base_url(),
        "test-key",
        "gpt-4o",
    );

    let req = CompletionRequest {
        model: "gpt-4o".into(),
        messages: vec![Message::user("hi")],
        ..Default::default()
    };

    let mut stream = client.complete(req).await.unwrap();
    let mut text = String::new();

    while let Some(delta) = stream.next().await {
        if let Ok(Delta::Text(t)) = delta { text.push_str(&t); }
    }

    assert_eq!(text, "Hello");
}
```

---

## 6. CI Configuration

```yaml
# .github/workflows/ci.yml
name: CI
on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - uses: Swatinem/rust-cache@v2

      - name: Format check
        run: cargo fmt --check

      - name: Clippy (deny warnings)
        run: cargo clippy --workspace -- -D warnings -D clippy::unwrap_used

      - name: Unit + Integration tests
        run: cargo test --workspace --exclude talon-e2e

      - name: Coverage
        run: |
          cargo install cargo-llvm-cov
          cargo llvm-cov --workspace --lcov --output-path lcov.info
        continue-on-error: true

  e2e:
    runs-on: ubuntu-latest
    if: github.ref == 'refs/heads/main'
    env:
      ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: E2E tests (real LLM)
        run: cargo test --package talon-e2e
```

---

## 7. Mocking Strategy

| Dependency | Mock Approach |
|------------|---------------|
| LLM provider | `MockLlmProvider` struct implementing `[LlmProvider](../05_API_Bindings/41_LLM_Provider_Abstraction.md)` trait |
| SQLite | In-memory `[rusqlite](../07_Memory_System/55_SQLite_FTS5_In_Rust.md)::Connection::open_in_memory()` |
| HTTP APIs | `httpmock::MockServer` |
| Filesystem | `tempfile::TempDir` |
| Docker | `MockDockerBackend` implementing `TerminalBackend` |
| Telegram | `MockGateway` implementing `Gateway` trait |

All traits are designed for mockability: `Send + Sync + 'static`, no concrete types in function signatures.
---

## Related Documents

### See Also
- [CI/CD Pipeline](../08_DevOps/62_CI_CD_Pipeline.md)
- [Error Handling Strategy](../06_Concurrency/54_Error_Handling_Strategy.md)
- [Build System / Cargo Workspace](../08_DevOps/60_Build_System_Cargo_Workspace.md)

