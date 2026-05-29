//! `WebSearchTool` — Brave Search API with a DuckDuckGo HTML fallback.

use std::{future::Future, pin::Pin};

use regex::Regex;
use serde_json::{Value, json};
use talon_core::{
    approval::ApprovalLevel,
    tools::{Tool, ToolContext, ToolResult},
};

const DEFAULT_COUNT: u32 = 5;
const MAX_COUNT: u32 = 20;

/// Searches the web. Tries the Brave Search API first (when `BRAVE_API_KEY` is
/// set) and falls back to DuckDuckGo's HTML endpoint otherwise.
///
/// `Safe` — read-only query.
pub struct WebSearchTool {
    client: reqwest::Client,
    brave_api_key: Option<String>,
    brave_base: String,
    ddg_base: String,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self::with_config(
            std::env::var("BRAVE_API_KEY")
                .ok()
                .filter(|k| !k.is_empty()),
            "https://api.search.brave.com".to_string(),
            "https://html.duckduckgo.com".to_string(),
        )
    }

    fn with_config(brave_api_key: Option<String>, brave_base: String, ddg_base: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            brave_api_key,
            brave_base,
            ddg_base,
        }
    }

    /// Query Brave. `Ok(None)` means "no usable result, try the fallback";
    /// `Err` is a hard transport error.
    async fn brave_search(&self, query: &str, count: u32) -> Result<Option<String>, String> {
        let key = match &self.brave_api_key {
            Some(k) => k,
            None => return Ok(None),
        };
        let url = format!("{}/res/v1/web/search", self.brave_base);
        let resp = self
            .client
            .get(&url)
            .header("X-Subscription-Token", key)
            .header("Accept", "application/json")
            .query(&[("q", query), ("count", &count.to_string())])
            .send()
            .await
            .map_err(|e| format!("brave request failed: {e}"))?;

        if !resp.status().is_success() {
            // Auth/quota errors → fall back rather than fail the tool.
            return Ok(None);
        }

        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("brave returned invalid JSON: {e}"))?;

        let results = body["web"]["results"].as_array();
        let Some(results) = results.filter(|r| !r.is_empty()) else {
            return Ok(None);
        };

        let mut out = format!("Web results for \"{query}\" (via Brave):\n");
        for (i, r) in results.iter().take(count as usize).enumerate() {
            let title = r["title"].as_str().unwrap_or("(no title)");
            let link = r["url"].as_str().unwrap_or("");
            let desc = r["description"].as_str().unwrap_or("");
            out.push_str(&format!("\n{}. {title}\n   {link}\n   {desc}\n", i + 1));
        }
        Ok(Some(out))
    }

    /// Query DuckDuckGo's HTML endpoint and scrape result anchors.
    async fn ddg_search(&self, query: &str, count: u32) -> Result<String, String> {
        let url = format!("{}/html/", self.ddg_base);
        let resp = self
            .client
            .get(&url)
            .query(&[("q", query)])
            .send()
            .await
            .map_err(|e| format!("duckduckgo request failed: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!(
                "duckduckgo returned HTTP {}",
                resp.status().as_u16()
            ));
        }

        let html = resp
            .text()
            .await
            .map_err(|e| format!("failed to read duckduckgo body: {e}"))?;

        // `<a ... class="result__a" href="URL">TITLE</a>`
        let anchor = Regex::new(r#"(?s)class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#)
            .map_err(|e| format!("internal regex error: {e}"))?;
        let tags = Regex::new(r"<[^>]+>").map_err(|e| format!("internal regex error: {e}"))?;

        let mut out = format!("Web results for \"{query}\" (via DuckDuckGo):\n");
        let mut n = 0;
        for cap in anchor.captures_iter(&html) {
            if n >= count {
                break;
            }
            let link = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            let raw_title = cap.get(2).map(|m| m.as_str()).unwrap_or("");
            let title = tags.replace_all(raw_title, "").trim().to_string();
            if title.is_empty() {
                continue;
            }
            n += 1;
            out.push_str(&format!("\n{n}. {title}\n   {link}\n"));
        }

        if n == 0 {
            return Err(format!("no results found for \"{query}\""));
        }
        Ok(out)
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "web_search",
            "description": "Search the web and return a ranked list of result titles, URLs, \
                            and snippets. Uses Brave Search when configured, else DuckDuckGo.",
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

            match self.brave_search(&query, count).await {
                Ok(Some(results)) => return ToolResult::ok(results),
                Ok(None) => {} // fall through to DuckDuckGo
                Err(e) => return ToolResult::err(e),
            }

            match self.ddg_search(&query, count).await {
                Ok(results) => ToolResult::ok(results),
                Err(e) => ToolResult::err(e),
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
                { "title": "Rust Lang", "url": "https://rust-lang.org", "description": "The Rust programming language." },
                { "title": "Tokio", "url": "https://tokio.rs", "description": "Async runtime." }
            ]}
        })
    }

    fn ddg_html() -> &'static str {
        r##"<html><body>
        <a rel="nofollow" class="result__a" href="https://example.com/a">First <b>Result</b></a>
        <a rel="nofollow" class="result__a" href="https://example.com/b">Second Result</a>
        </body></html>"##
    }

    #[tokio::test]
    async fn brave_success_returns_results() {
        let brave = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(brave_body()))
            .mount(&brave)
            .await;

        let tool = WebSearchTool::with_config(
            Some("test-key".to_string()),
            brave.uri(),
            "http://unused.invalid".to_string(),
        );
        let r = tool
            .execute(json!({ "query": "rust" }), ToolContext::default())
            .await;

        assert!(!r.is_error, "got error: {}", r.content);
        assert!(r.content.contains("Brave"));
        assert!(r.content.contains("Rust Lang"));
        assert!(r.content.contains("https://tokio.rs"));
    }

    #[tokio::test]
    async fn brave_401_falls_back_to_ddg() {
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

        let tool = WebSearchTool::with_config(Some("bad-key".to_string()), brave.uri(), ddg.uri());
        let r = tool
            .execute(json!({ "query": "rust" }), ToolContext::default())
            .await;

        assert!(!r.is_error, "got error: {}", r.content);
        assert!(r.content.contains("DuckDuckGo"));
        assert!(
            r.content.contains("First Result"),
            "tags not stripped: {}",
            r.content
        );
        assert!(r.content.contains("https://example.com/b"));
    }

    #[tokio::test]
    async fn no_key_uses_ddg() {
        let ddg = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/html/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(ddg_html()))
            .mount(&ddg)
            .await;

        let tool = WebSearchTool::with_config(None, "http://unused.invalid".to_string(), ddg.uri());
        let r = tool
            .execute(json!({ "query": "rust" }), ToolContext::default())
            .await;

        assert!(!r.is_error, "got error: {}", r.content);
        assert!(r.content.contains("DuckDuckGo"));
    }

    #[tokio::test]
    async fn missing_query_is_failure() {
        let tool = WebSearchTool::with_config(None, "x".into(), "y".into());
        let r = tool.execute(json!({}), ToolContext::default()).await;
        assert!(r.is_error);
        assert!(r.content.contains("query"));
    }

    #[test]
    fn metadata_is_safe_and_named() {
        let tool = WebSearchTool::with_config(None, "x".into(), "y".into());
        assert_eq!(tool.name(), "web_search");
        assert_eq!(tool.approval_level(&Value::Null), ApprovalLevel::Safe);
        assert_eq!(tool.schema()["name"], "web_search");
    }
}
