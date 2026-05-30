//! `WebSearchTool` — runs an ordered chain of [`SearchBackend`]s, using the
//! first that returns results (Brave → DuckDuckGo by default).

use std::{future::Future, pin::Pin};

use serde_json::{Value, json};
use talon_core::{
    approval::ApprovalLevel,
    tools::{Tool, ToolContext, ToolResult},
};

use crate::web::backend::{BraveBackend, DdgBackend, SearchBackend, SearchResult};

const DEFAULT_COUNT: u32 = 5;
const MAX_COUNT: u32 = 20;

/// Searches the web through a configurable backend chain. Each backend is tried
/// in order; the first to return a non-empty result set wins.
///
/// `Safe` — read-only query.
pub struct WebSearchTool {
    backends: Vec<Box<dyn SearchBackend>>,
}

impl WebSearchTool {
    /// Default chain: Brave (if `BRAVE_API_KEY` set) → DuckDuckGo.
    pub fn new() -> Self {
        Self {
            backends: vec![
                Box::new(BraveBackend::from_env()),
                Box::new(DdgBackend::default_base()),
            ],
        }
    }

    /// Construct with an explicit backend chain (used by callers wiring config,
    /// and by tests).
    pub fn with_backends(backends: Vec<Box<dyn SearchBackend>>) -> Self {
        Self { backends }
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

fn format_results(backend: &str, query: &str, results: &[SearchResult]) -> String {
    let mut out = format!("Web results for \"{query}\" (via {backend}):\n");
    for (i, r) in results.iter().enumerate() {
        out.push_str(&format!("\n{}. {}\n   {}\n", i + 1, r.title, r.url));
        if !r.snippet.is_empty() {
            out.push_str(&format!("   {}\n", r.snippet));
        }
    }
    out
}

impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "web_search",
            "description": "Search the web and return a ranked list of result titles, URLs, \
                            and snippets. Tries configured backends in order (Brave, SearXNG, \
                            DuckDuckGo, …).",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The search query." },
                    "count": {
                        "type": "integer",
                        "description": "Max results to return (1-20, default 5)."
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
        Box::pin(async move {
            let query = match args["query"].as_str() {
                Some(q) if !q.trim().is_empty() => q.trim().to_string(),
                _ => return ToolResult::err("missing required argument: query"),
            };
            let count = args["count"]
                .as_u64()
                .map(|c| (c as u32).clamp(1, MAX_COUNT))
                .unwrap_or(DEFAULT_COUNT);

            let mut last_err = None;
            for backend in &self.backends {
                match backend.search(&query, count).await {
                    Ok(results) if !results.is_empty() => {
                        return ToolResult::ok(format_results(backend.name(), &query, &results));
                    }
                    Ok(_) => {} // empty → try the next backend
                    Err(e) => {
                        tracing::warn!(backend = backend.name(), error = %e, "search backend failed");
                        last_err = Some(e);
                    }
                }
            }

            match last_err {
                Some(e) => ToolResult::err(format!("all search backends failed: {e}")),
                None => ToolResult::err(format!("no results found for \"{query}\"")),
            }
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn brave_body() -> Value {
        json!({
            "web": { "results": [
                { "title": "Rust Lang", "url": "https://rust-lang.org", "description": "The Rust programming language." }
            ]}
        })
    }

    fn ddg_html() -> &'static str {
        r##"<a class="result__a" href="https://example.com/a">DDG Result</a>"##
    }

    #[tokio::test]
    async fn first_backend_with_results_wins() {
        let brave = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(brave_body()))
            .mount(&brave)
            .await;

        let tool = WebSearchTool::with_backends(vec![Box::new(BraveBackend::new(
            Some("k".into()),
            brave.uri(),
        ))]);
        let r = tool
            .execute(json!({ "query": "rust" }), ToolContext::default())
            .await;
        assert!(!r.is_error, "got: {}", r.content);
        assert!(r.content.contains("brave"));
        assert!(r.content.contains("Rust Lang"));
    }

    #[tokio::test]
    async fn falls_through_to_next_backend() {
        // Brave returns 401 → chain moves to DDG.
        let brave = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&brave)
            .await;
        let ddg = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/html/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(ddg_html()))
            .mount(&ddg)
            .await;

        let tool = WebSearchTool::with_backends(vec![
            Box::new(BraveBackend::new(Some("bad".into()), brave.uri())),
            Box::new(DdgBackend::new(ddg.uri())),
        ]);
        let r = tool
            .execute(json!({ "query": "rust" }), ToolContext::default())
            .await;
        assert!(!r.is_error, "got: {}", r.content);
        assert!(r.content.contains("duckduckgo"));
        assert!(r.content.contains("DDG Result"));
    }

    #[tokio::test]
    async fn unavailable_backend_is_skipped() {
        // Brave has no key (Unavailable) → DDG used.
        let ddg = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/html/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(ddg_html()))
            .mount(&ddg)
            .await;

        let tool = WebSearchTool::with_backends(vec![
            Box::new(BraveBackend::new(None, "http://unused.invalid")),
            Box::new(DdgBackend::new(ddg.uri())),
        ]);
        let r = tool
            .execute(json!({ "query": "rust" }), ToolContext::default())
            .await;
        assert!(!r.is_error, "got: {}", r.content);
        assert!(r.content.contains("DDG Result"));
    }

    #[tokio::test]
    async fn missing_query_is_failure() {
        let tool = WebSearchTool::new();
        let r = tool.execute(json!({}), ToolContext::default()).await;
        assert!(r.is_error);
        assert!(r.content.contains("query"));
    }

    #[test]
    fn metadata_is_safe_and_named() {
        let tool = WebSearchTool::new();
        assert_eq!(tool.name(), "web_search");
        assert_eq!(tool.approval_level(&Value::Null), ApprovalLevel::Safe);
        assert_eq!(tool.schema()["name"], "web_search");
    }
}
