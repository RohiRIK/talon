//! Browser tool (Phase 5, experimental — `feature = "browser"`).
//!
//! Drives a headless Chrome instance via `headless_chrome` (CDP). Behind a
//! feature flag and off by default: it pulls a heavy native dependency and
//! needs a Chrome binary at runtime.

pub mod pool;

use std::sync::Arc;
use std::{future::Future, pin::Pin};

use serde_json::{Value, json};
use talon_core::{
    approval::ApprovalLevel,
    tools::{Tool, ToolContext, ToolResult},
};

pub use pool::BrowserPool;

/// Opens a URL in headless Chrome and returns the rendered page HTML.
///
/// `NeedsApproval` — drives a real browser. Shares a [`BrowserPool`] so the
/// Chrome process is launched once and reused.
pub struct BrowserTool {
    pool: Arc<BrowserPool>,
}

impl BrowserTool {
    pub fn new(pool: Arc<BrowserPool>) -> Self {
        Self { pool }
    }
}

impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser_open"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "browser_open",
            "description": "Open a URL in a headless browser and return the fully rendered page \
                            HTML (after JavaScript executes).",
            "input_schema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The absolute URL to open." }
                },
                "required": ["url"]
            }
        })
    }

    fn approval_level(&self, _args: &Value) -> ApprovalLevel {
        ApprovalLevel::NeedsApproval
    }

    fn execute(
        &self,
        args: Value,
        _ctx: ToolContext,
    ) -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>> {
        let pool = Arc::clone(&self.pool);
        Box::pin(async move {
            let url = match args["url"].as_str() {
                Some(u) if !u.is_empty() => u.to_string(),
                _ => return ToolResult::err("missing required argument: url"),
            };
            match pool.fetch_content(url).await {
                Ok(html) => ToolResult::ok(html),
                Err(e) => ToolResult::err(e),
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn metadata_needs_approval_and_named() {
        let tool = BrowserTool::new(Arc::new(BrowserPool::new(1)));
        assert_eq!(tool.name(), "browser_open");
        assert_eq!(
            tool.approval_level(&Value::Null),
            ApprovalLevel::NeedsApproval
        );
        assert_eq!(tool.schema()["name"], "browser_open");
    }

    #[tokio::test]
    async fn missing_url_is_failure() {
        // Arg validation happens before any browser launch — no Chrome needed.
        let tool = BrowserTool::new(Arc::new(BrowserPool::new(1)));
        let r = tool.execute(json!({}), ToolContext::default()).await;
        assert!(r.is_error);
        assert!(r.content.contains("url"));
    }
}
