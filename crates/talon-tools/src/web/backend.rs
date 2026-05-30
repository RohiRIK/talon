//! Pluggable search backends (Phase 5.1).
//!
//! `WebSearchTool` holds an ordered chain of `SearchBackend`s and uses the
//! first that returns results. Backends normalize every engine's response to a
//! common [`SearchResult`]. No `async-trait` — the trait returns a boxed future
//! to stay object-safe (same pattern as `Tool`, ADR 0007).

use std::future::Future;
use std::pin::Pin;

use regex::Regex;
use serde_json::Value;
use thiserror::Error;

/// One normalized search hit.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Error)]
pub enum SearchError {
    /// Backend can't run (e.g. missing API key) — the chain skips it.
    #[error("{0}")]
    Unavailable(String),
    #[error("transport: {0}")]
    Transport(String),
    #[error("parse: {0}")]
    Parse(String),
}

/// A single search engine. `Send + Sync` so the tool can hold `Box<dyn …>`.
pub trait SearchBackend: Send + Sync {
    fn name(&self) -> &str;
    fn search<'a>(
        &'a self,
        query: &'a str,
        count: u32,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchResult>, SearchError>> + Send + 'a>>;
}

// ── Brave ─────────────────────────────────────────────────────────────────────

/// Brave Search API. `Unavailable` when no key is configured.
pub struct BraveBackend {
    client: reqwest::Client,
    api_key: Option<String>,
    base: String,
}

impl BraveBackend {
    pub fn new(api_key: Option<String>, base: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base: base.into(),
        }
    }

    /// Default cloud endpoint, key from `BRAVE_API_KEY`.
    pub fn from_env() -> Self {
        Self::new(
            std::env::var("BRAVE_API_KEY")
                .ok()
                .filter(|k| !k.is_empty()),
            "https://api.search.brave.com",
        )
    }
}

impl SearchBackend for BraveBackend {
    fn name(&self) -> &str {
        "brave"
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        count: u32,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchResult>, SearchError>> + Send + 'a>> {
        Box::pin(async move {
            let key = self
                .api_key
                .as_ref()
                .ok_or_else(|| SearchError::Unavailable("no BRAVE_API_KEY".to_string()))?;
            let resp = self
                .client
                .get(format!("{}/res/v1/web/search", self.base))
                .header("X-Subscription-Token", key)
                .header("Accept", "application/json")
                .query(&[("q", query), ("count", &count.to_string())])
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
            let results = body["web"]["results"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .take(count as usize)
                        .map(|r| SearchResult {
                            title: r["title"].as_str().unwrap_or("").to_string(),
                            url: r["url"].as_str().unwrap_or("").to_string(),
                            snippet: r["description"].as_str().unwrap_or("").to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(results)
        })
    }
}

// ── DuckDuckGo ────────────────────────────────────────────────────────────────

/// DuckDuckGo via its HTML endpoint (no API key; scraped, last-resort fallback).
pub struct DdgBackend {
    client: reqwest::Client,
    base: String,
}

impl DdgBackend {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base: base.into(),
        }
    }

    pub fn default_base() -> Self {
        Self::new("https://html.duckduckgo.com")
    }
}

impl SearchBackend for DdgBackend {
    fn name(&self) -> &str {
        "duckduckgo"
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        count: u32,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchResult>, SearchError>> + Send + 'a>> {
        Box::pin(async move {
            let resp = self
                .client
                .get(format!("{}/html/", self.base))
                .query(&[("q", query)])
                .send()
                .await
                .map_err(|e| SearchError::Transport(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(SearchError::Transport(format!(
                    "HTTP {}",
                    resp.status().as_u16()
                )));
            }
            let html = resp
                .text()
                .await
                .map_err(|e| SearchError::Transport(e.to_string()))?;

            let anchor = Regex::new(r#"(?s)class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#)
                .map_err(|e| SearchError::Parse(e.to_string()))?;
            let tags = Regex::new(r"<[^>]+>").map_err(|e| SearchError::Parse(e.to_string()))?;

            let mut results = Vec::new();
            for cap in anchor.captures_iter(&html) {
                if results.len() >= count as usize {
                    break;
                }
                let url = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
                let raw_title = cap.get(2).map(|m| m.as_str()).unwrap_or("");
                let title = tags.replace_all(raw_title, "").trim().to_string();
                if title.is_empty() {
                    continue;
                }
                results.push(SearchResult {
                    title,
                    url,
                    snippet: String::new(),
                });
            }
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
    async fn brave_without_key_is_unavailable() {
        let b = BraveBackend::new(None, "http://unused.invalid");
        let err = b.search("x", 5).await.unwrap_err();
        assert!(matches!(err, SearchError::Unavailable(_)));
    }

    #[tokio::test]
    async fn brave_parses_results() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "web": { "results": [
                    { "title": "Rust", "url": "https://rust-lang.org", "description": "lang" }
                ]}
            })))
            .mount(&server)
            .await;
        let b = BraveBackend::new(Some("k".into()), server.uri());
        let r = b.search("rust", 5).await.unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].url, "https://rust-lang.org");
    }

    #[tokio::test]
    async fn ddg_scrapes_anchors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/html/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r##"<a class="result__a" href="https://example.com/a">First <b>Hit</b></a>"##,
            ))
            .mount(&server)
            .await;
        let d = DdgBackend::new(server.uri());
        let r = d.search("x", 5).await.unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "First Hit");
        assert_eq!(r[0].url, "https://example.com/a");
    }
}
