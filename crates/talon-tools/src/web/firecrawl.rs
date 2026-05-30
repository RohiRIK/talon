//! Firecrawl backend — search (`/v1/search`) and JS-rendered scrape
//! (`/v1/scrape`). Cloud (`api.firecrawl.dev`, needs `FIRECRAWL_API_KEY`) or
//! self-hosted (`FIRECRAWL_API_URL`, key optional). Request shapes verified
//! against Open WebUI and `NousResearch/hermes-agent`.

use std::future::Future;
use std::pin::Pin;

use serde_json::{Value, json};

use crate::web::backend::{SearchBackend, SearchError, SearchResult, result_from, send_json};

const CLOUD_BASE: &str = "https://api.firecrawl.dev";

pub struct FirecrawlBackend {
    client: reqwest::Client,
    api_key: Option<String>,
    base: String,
}

impl FirecrawlBackend {
    pub fn new(api_key: Option<String>, base: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base: base.into(),
        }
    }

    /// Key from `FIRECRAWL_API_KEY`, base from `FIRECRAWL_API_URL` (self-host)
    /// else the cloud endpoint.
    pub fn from_env() -> Self {
        let base = std::env::var("FIRECRAWL_API_URL")
            .ok()
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| CLOUD_BASE.to_string());
        Self::new(
            std::env::var("FIRECRAWL_API_KEY")
                .ok()
                .filter(|k| !k.is_empty()),
            base,
        )
    }

    /// Cloud requires a key; a self-hosted instance may not.
    fn ensure_available(&self) -> Result<(), SearchError> {
        if self.api_key.is_none() && self.base.contains("api.firecrawl.dev") {
            return Err(SearchError::Unavailable("no FIRECRAWL_API_KEY".to_string()));
        }
        Ok(())
    }

    fn post(&self, path: &str, body: Value) -> reqwest::RequestBuilder {
        let req = self.client.post(format!("{}{path}", self.base)).json(&body);
        match &self.api_key {
            Some(k) => req.bearer_auth(k),
            None => req,
        }
    }

    /// Scrape a URL and return its rendered markdown. Used by the fetch chain.
    pub async fn scrape(&self, url: &str) -> Result<String, SearchError> {
        self.ensure_available()?;
        let body =
            send_json(self.post("/v1/scrape", json!({ "url": url, "formats": ["markdown"] })))
                .await?;
        match body["data"]["markdown"].as_str() {
            Some(md) if !md.is_empty() => Ok(md.to_string()),
            _ => Err(SearchError::Parse(
                "firecrawl returned no markdown".to_string(),
            )),
        }
    }
}

impl SearchBackend for FirecrawlBackend {
    fn name(&self) -> &str {
        "firecrawl"
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        count: u32,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchResult>, SearchError>> + Send + 'a>> {
        Box::pin(async move {
            self.ensure_available()?;
            let body =
                send_json(self.post("/v1/search", json!({ "query": query, "limit": count })))
                    .await?;
            let results = body["data"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .take(count as usize)
                        .map(|r| result_from(r, "description"))
                        .collect()
                })
                .unwrap_or_default();
            Ok(results)
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn cloud_without_key_is_unavailable() {
        let b = FirecrawlBackend::new(None, CLOUD_BASE);
        assert!(matches!(
            b.search("x", 5).await.unwrap_err(),
            SearchError::Unavailable(_)
        ));
        assert!(matches!(
            b.scrape("https://example.com").await.unwrap_err(),
            SearchError::Unavailable(_)
        ));
    }

    #[tokio::test]
    async fn search_parses_data_array() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "data": [
                    { "url": "https://rust-lang.org", "title": "Rust", "description": "lang" }
                ]
            })))
            .mount(&server)
            .await;
        let b = FirecrawlBackend::new(Some("fc-key".into()), server.uri());
        let r = b.search("rust", 5).await.unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].url, "https://rust-lang.org");
    }

    #[tokio::test]
    async fn scrape_returns_markdown() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/scrape"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "data": { "markdown": "# Hello\n\nbody", "metadata": {} }
            })))
            .mount(&server)
            .await;
        // self-hosted base (no api.firecrawl.dev) → key optional
        let b = FirecrawlBackend::new(None, server.uri());
        let md = b.scrape("https://example.com").await.unwrap();
        assert!(md.contains("Hello"));
    }
}
