use std::{future::Future, pin::Pin, sync::Arc};

use serde_json::{Value, json};
use talon_core::{
    approval::ApprovalLevel,
    tools::{Tool, ToolContext, ToolResult},
};
use talon_memory::MemoryStore;

/// Search past conversation messages with FTS5.
///
/// ApprovalLevel::Safe — reads only, no side effects.
pub struct SessionSearchTool {
    store: Arc<dyn MemoryStore>,
}

impl SessionSearchTool {
    pub fn new(store: Arc<dyn MemoryStore>) -> Self {
        Self { store }
    }
}

impl Tool for SessionSearchTool {
    fn name(&self) -> &str {
        "session_search"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "session_search",
            "description": "Search past conversation messages using full-text search. Returns the most relevant messages matching the query.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results to return (default: 5).",
                        "default": 5
                    }
                },
                "required": ["query"]
            }
        })
    }

    fn approval_level(&self, _args: &Value) -> ApprovalLevel {
        ApprovalLevel::Safe
    }

    fn execute(
        &self,
        args: Value,
        _ctx: ToolContext,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let store = Arc::clone(&self.store);
        Box::pin(async move {
            let query = match args["query"].as_str() {
                Some(q) if !q.is_empty() => q.to_string(),
                _ => return ToolResult::err("missing required argument: query"),
            };
            let limit = args["limit"].as_u64().unwrap_or(5) as usize;

            match store.search_messages(&query, limit).await {
                Ok(msgs) if msgs.is_empty() => {
                    ToolResult::ok("No matching messages found.")
                }
                Ok(msgs) => {
                    let lines: Vec<String> = msgs
                        .iter()
                        .map(|m| format!("[{}] {}: {}", m.created_at, m.role, m.content))
                        .collect();
                    ToolResult::ok(lines.join("\n"))
                }
                Err(e) => ToolResult::err(format!("search failed: {e}")),
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;

    use talon_memory::{Database, SqliteStore};

    use super::*;

    async fn make_tool() -> SessionSearchTool {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        let store: Arc<dyn MemoryStore> = Arc::new(SqliteStore::new(Arc::clone(&db)));
        SessionSearchTool::new(store)
    }

    #[test]
    fn tool_name_is_session_search() {
        let tool = SessionSearchTool::new(Arc::new(talon_memory::SqliteStore::new(Arc::new(
            Database::open(":memory:").expect("open"),
        ))));
        assert_eq!(tool.name(), "session_search");
    }

    #[test]
    fn tool_approval_level_is_safe() {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        let store: Arc<dyn MemoryStore> = Arc::new(SqliteStore::new(db));
        let tool = SessionSearchTool::new(store);
        assert_eq!(tool.approval_level(&Value::Null), ApprovalLevel::Safe);
    }

    #[test]
    fn schema_contains_required_fields() {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        let store: Arc<dyn MemoryStore> = Arc::new(SqliteStore::new(db));
        let tool = SessionSearchTool::new(store);
        let s = tool.schema();
        assert_eq!(s["name"], "session_search");
        assert!(s["input_schema"]["properties"]["query"].is_object());
        assert_eq!(s["input_schema"]["required"][0], "query");
    }

    #[tokio::test]
    async fn execute_missing_query_returns_error() {
        let tool = make_tool().await;
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(result.is_error);
        assert!(result.content.contains("query"));
    }

    #[tokio::test]
    async fn execute_no_results_returns_no_match_message() {
        let tool = make_tool().await;
        let result = tool
            .execute(json!({"query": "xyzzy_nonexistent"}), ToolContext::default())
            .await;
        assert!(!result.is_error);
        assert!(result.content.contains("No matching"));
    }

    #[tokio::test]
    async fn execute_returns_matching_messages() {
        let db = Arc::new(Database::open(":memory:").expect("open"));
        db.init_schema().await.expect("schema");
        db.save_message("s1", "user", "Rust memory safety")
            .await
            .expect("save");
        let store: Arc<dyn MemoryStore> = Arc::new(SqliteStore::new(Arc::clone(&db)));
        let tool = SessionSearchTool::new(store);

        let result = tool
            .execute(json!({"query": "memory", "limit": 5}), ToolContext::default())
            .await;
        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert!(result.content.contains("Rust memory safety"));
    }
}
