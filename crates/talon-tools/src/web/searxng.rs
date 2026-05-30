//! SearXNG search backend — self-hosted, keyless meta-search.
//!
//! SearXNG aggregates other engines (Google, Bing, DuckDuckGo, …) behind one
//! endpoint you run yourself. Querying needs `format=json` enabled in the
//! instance's `settings.yml` (many public instances disable it).

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::web::backend::{SearchBackend, SearchError, SearchResult};

pub struct SearxngBackend {
    client: reqwest::Client,
    base: String,
}

impl SearxngBackend {
    /// `base` is the instance root, e.g. `http://localhost:8080`.
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base: base.into(),
        }
    }
}

impl SearchBackend for SearxngBackend {
    fn name(&self) -> &str {
        "searxng"
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        count: u32,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchResult>, SearchError>> + Send + 'a>> {
        Box::pin(async move {
            let resp = self
                .client
                .get(format!("{}/search", self.base))
                .query(&[("q", query), ("format", "json"), ("pageno", "1")])
                .send()
                .await
                .map_err(|e| SearchError::Transport(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(SearchError::Transport(format!(
                    "HTTP {}",
                    resp.status().as_u16()
                )));
            }
            let body: Value = resp
                .json()
                .await
                .map_err(|e| SearchError::Parse(e.to_string()))?;
            let results = body["results"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .take(count as usize)
                        .map(|r| SearchResult {
                            title: r["title"].as_str().unwrap_or("").to_string(),
                            url: r["url"].as_str().unwrap_or("").to_string(),
                            snippet: r["content"].as_str().unwrap_or("").to_string(),
                        })
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
    async fn parses_json_results() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    { "title": "Rust", "url": "https://rust-lang.org", "content": "systems language" },
                    { "title": "Tokio", "url": "https://tokio.rs", "content": "async runtime" }
                ]
            })))
            .mount(&server)
            .await;

        let b = SearxngBackend::new(server.uri());
        let r = b.search("rust", 5).await.unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].url, "https://rust-lang.org");
        assert_eq!(r[1].snippet, "async runtime");
    }

    #[tokio::test]
    async fn non_200_is_transport_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let b = SearxngBackend::new(server.uri());
        assert!(b.search("x", 5).await.is_err());
    }
}
