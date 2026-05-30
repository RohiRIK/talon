//! `WebExtractTool` — turn a URL into readable content through an ordered
//! [`FetchBackend`] chain (native Rust floor → browser/firecrawl escalation).

use std::{future::Future, pin::Pin};

use serde_json::{Value, json};
use talon_core::{
    approval::ApprovalLevel,
    tools::{Tool, ToolContext, ToolResult},
};

use crate::web::fetch::{FetchBackend, NativeFetch};

/// Fetches a web page and returns its main readable content. Tries each
/// configured backend in order; the first non-empty result wins.
///
/// `Safe` — read-only HTTP GET.
pub struct WebExtractTool {
    backends: Vec<Box<dyn FetchBackend>>,
}

impl WebExtractTool {
    /// Default chain: native Rust (reqwest + readability) only.
    pub fn new() -> Self {
        Self {
            backends: vec![Box::new(NativeFetch::new())],
        }
    }

    /// Construct with an explicit fetch chain (config wiring + tests).
    pub fn with_backends(backends: Vec<Box<dyn FetchBackend>>) -> Self {
        Self { backends }
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
        Box::pin(async move {
            let url = match args["url"].as_str() {
                Some(u) if !u.is_empty() => u.to_string(),
                _ => return ToolResult::err("missing required argument: url"),
            };

            let mut last_err = None;
            for backend in &self.backends {
                match backend.fetch(&url).await {
                    Ok(content) if !content.is_empty() => return ToolResult::ok(content),
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(backend = backend.name(), error = %e, "fetch backend failed");
                        last_err = Some(e);
                    }
                }
            }

            match last_err {
                Some(e) => ToolResult::err(format!("could not fetch '{url}': {e}")),
                None => ToolResult::err(format!("no content for '{url}'")),
            }
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::web::fetch::FetchError;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn article_html() -> String {
        let para = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod \
            tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis \
            nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.";
        format!(
            "<!DOCTYPE html><html><head><title>Test Article</title></head><body>\
             <nav>Home About Contact Login Signup</nav>\
             <article><h1>The Headline</h1>\
             <p>{para}</p><p>{para}</p><p>{para}</p><p>{para}</p></article>\
             <footer>Copyright 2026</footer></body></html>"
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
        assert!(r.content.contains("Lorem ipsum"));
        assert!(!r.content.contains("Home About Contact"));
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
        assert!(r.content.contains("500"), "got: {}", r.content);
    }

    #[tokio::test]
    async fn missing_url_is_failure() {
        let tool = WebExtractTool::new();
        let r = tool.execute(json!({}), ToolContext::default()).await;
        assert!(r.is_error);
        assert!(r.content.contains("url"));
    }

    /// A failing native floor escalates to the next backend in the chain.
    #[tokio::test]
    async fn escalates_to_next_backend() {
        struct FailFetch;
        impl FetchBackend for FailFetch {
            fn name(&self) -> &str {
                "fail"
            }
            fn fetch<'a>(
                &'a self,
                _url: &'a str,
            ) -> Pin<Box<dyn Future<Output = Result<String, FetchError>> + Send + 'a>> {
                Box::pin(async { Err(FetchError::Empty("nope".into())) })
            }
        }
        struct OkFetch;
        impl FetchBackend for OkFetch {
            fn name(&self) -> &str {
                "ok"
            }
            fn fetch<'a>(
                &'a self,
                _url: &'a str,
            ) -> Pin<Box<dyn Future<Output = Result<String, FetchError>> + Send + 'a>> {
                Box::pin(async { Ok("escalated content".to_string()) })
            }
        }

        let tool = WebExtractTool::with_backends(vec![Box::new(FailFetch), Box::new(OkFetch)]);
        let r = tool
            .execute(json!({ "url": "https://x.test" }), ToolContext::default())
            .await;
        assert!(!r.is_error, "got: {}", r.content);
        assert_eq!(r.content, "escalated content");
    }

    #[test]
    fn metadata_is_safe_and_named() {
        let tool = WebExtractTool::new();
        assert_eq!(tool.name(), "web_extract");
        assert_eq!(tool.approval_level(&Value::Null), ApprovalLevel::Safe);
        assert_eq!(tool.schema()["name"], "web_extract");
    }
}
