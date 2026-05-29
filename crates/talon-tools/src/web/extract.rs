//! `WebExtractTool` — fetch a URL and return its readable article text.

use std::{future::Future, pin::Pin};

use dom_smoothie::{Config, Readability, TextMode};
use serde_json::{Value, json};
use talon_core::{
    approval::ApprovalLevel,
    tools::{Tool, ToolContext, ToolResult},
};

/// Max HTML bytes to buffer before bailing — guards against huge pages.
const MAX_BYTES: usize = 5 * 1024 * 1024; // 5 MB

/// Fetches a web page and returns its main readable content, with navigation,
/// ads, and scripts stripped via a Readability port (`dom_smoothie`).
///
/// `Safe` — read-only HTTP GET.
pub struct WebExtractTool {
    client: reqwest::Client,
}

impl WebExtractTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for WebExtractTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for WebExtractTool {
    fn name(&self) -> &str {
        "web_extract"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "web_extract",
            "description": "Fetch a web page and return its main readable text content, \
                            with navigation, ads, and scripts removed.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The absolute URL to fetch (http or https)."
                    }
                },
                "required": ["url"]
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
        let client = self.client.clone();
        Box::pin(async move {
            let url = match args["url"].as_str() {
                Some(u) if !u.is_empty() => u.to_string(),
                _ => return ToolResult::err("missing required argument: url"),
            };

            let resp = match client.get(&url).send().await {
                Ok(r) => r,
                Err(e) => return ToolResult::err(format!("request to '{url}' failed: {e}")),
            };

            if !resp.status().is_success() {
                return ToolResult::err(format!(
                    "'{url}' returned HTTP {}",
                    resp.status().as_u16()
                ));
            }

            // Reject oversized pages before buffering the whole body.
            if let Some(len) = resp.content_length()
                && len as usize > MAX_BYTES
            {
                return ToolResult::err(format!(
                    "'{url}' body is {len} bytes — exceeds 5 MB limit"
                ));
            }

            let html = match resp.text().await {
                Ok(b) => b,
                Err(e) => return ToolResult::err(format!("failed to read body of '{url}': {e}")),
            };

            if html.len() > MAX_BYTES {
                return ToolResult::err(format!("'{url}' body exceeds 5 MB limit"));
            }

            let cfg = Config {
                text_mode: TextMode::Formatted,
                ..Default::default()
            };

            let mut readability =
                match Readability::new(html.as_str(), Some(url.as_str()), Some(cfg)) {
                    Ok(r) => r,
                    Err(e) => return ToolResult::err(format!("could not parse '{url}': {e}")),
                };

            match readability.parse() {
                Ok(article) => {
                    let title = article.title.trim();
                    let body = article.text_content.trim();
                    let out = if title.is_empty() {
                        body.to_string()
                    } else {
                        format!("# {title}\n\n{body}")
                    };
                    ToolResult::ok(out)
                }
                Err(e) => ToolResult::err(format!(
                    "could not extract readable content from '{url}': {e}"
                )),
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

    /// A page with enough article prose for Readability to "grab" it,
    /// wrapped in nav/footer boilerplate that should be stripped.
    fn article_html() -> String {
        let para = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod \
            tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis \
            nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.";
        format!(
            "<!DOCTYPE html><html><head><title>Test Article</title></head><body>\
             <nav>Home About Contact Login Signup</nav>\
             <article><h1>The Headline</h1>\
             <p>{para}</p><p>{para}</p><p>{para}</p><p>{para}</p></article>\
             <footer>Copyright 2026 — all rights reserved</footer></body></html>"
        )
    }

    #[tokio::test]
    async fn extracts_readable_text() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/article"))
            .respond_with(ResponseTemplate::new(200).set_body_string(article_html()))
            .mount(&server)
            .await;

        let tool = WebExtractTool::new();
        let r = tool
            .execute(
                json!({ "url": format!("{}/article", server.uri()) }),
                ToolContext::default(),
            )
            .await;

        assert!(!r.is_error, "expected success, got: {}", r.content);
        assert!(
            r.content.contains("Lorem ipsum"),
            "readable body missing: {}",
            r.content
        );
        // Boilerplate must be stripped.
        assert!(
            !r.content.contains("Home About Contact"),
            "nav not stripped"
        );
    }

    #[tokio::test]
    async fn http_error_is_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let tool = WebExtractTool::new();
        let r = tool
            .execute(
                json!({ "url": format!("{}/x", server.uri()) }),
                ToolContext::default(),
            )
            .await;

        assert!(r.is_error);
        assert!(
            r.content.contains("500"),
            "expected HTTP 500 in: {}",
            r.content
        );
    }

    #[tokio::test]
    async fn missing_url_is_failure() {
        let tool = WebExtractTool::new();
        let r = tool.execute(json!({}), ToolContext::default()).await;
        assert!(r.is_error);
        assert!(r.content.contains("url"));
    }

    #[test]
    fn metadata_is_safe_and_named() {
        let tool = WebExtractTool::new();
        assert_eq!(tool.name(), "web_extract");
        assert_eq!(tool.approval_level(&Value::Null), ApprovalLevel::Safe);
        assert_eq!(tool.schema()["name"], "web_extract");
    }
}
