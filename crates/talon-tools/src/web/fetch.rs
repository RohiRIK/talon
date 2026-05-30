//! Pluggable fetch backends (Phase 5.1).
//!
//! `WebExtractTool` runs an ordered chain: native Rust (`reqwest` +
//! `dom_smoothie`) is the dependency-free floor; `browser` (headless Chrome)
//! and `firecrawl` are escalations for JS-heavy / hard pages. The chain uses
//! the first backend that returns non-empty content.

use std::future::Future;
use std::pin::Pin;

use dom_smoothie::{Config, Readability, TextMode};
use thiserror::Error;

use crate::web::backend::SearchError;
use crate::web::firecrawl::FirecrawlBackend;

/// Max HTML bytes to buffer before bailing.
const MAX_BYTES: usize = 5 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum FetchError {
    /// Backend can't run (e.g. missing key) — the chain skips it.
    #[error("{0}")]
    Unavailable(String),
    #[error("transport: {0}")]
    Transport(String),
    #[error("no usable content: {0}")]
    Empty(String),
    #[error("parse: {0}")]
    Parse(String),
}

impl From<SearchError> for FetchError {
    fn from(e: SearchError) -> Self {
        match e {
            SearchError::Unavailable(m) => FetchError::Unavailable(m),
            SearchError::Transport(m) => FetchError::Transport(m),
            SearchError::Parse(m) => FetchError::Parse(m),
        }
    }
}

/// A way to turn a URL into readable content.
pub trait FetchBackend: Send + Sync {
    fn name(&self) -> &str;
    fn fetch<'a>(
        &'a self,
        url: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, FetchError>> + Send + 'a>>;
}

// ── Native (reqwest + dom_smoothie) — the floor ───────────────────────────────

pub struct NativeFetch {
    client: reqwest::Client,
}

impl NativeFetch {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for NativeFetch {
    fn default() -> Self {
        Self::new()
    }
}

impl FetchBackend for NativeFetch {
    fn name(&self) -> &str {
        "native"
    }

    fn fetch<'a>(
        &'a self,
        url: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, FetchError>> + Send + 'a>> {
        Box::pin(async move {
            let resp = self
                .client
                .get(url)
                .send()
                .await
                .map_err(|e| FetchError::Transport(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(FetchError::Transport(format!(
                    "HTTP {}",
                    resp.status().as_u16()
                )));
            }
            if let Some(len) = resp.content_length()
                && len as usize > MAX_BYTES
            {
                return Err(FetchError::Transport(format!(
                    "body {len} bytes exceeds 5 MB"
                )));
            }
            let html = resp
                .text()
                .await
                .map_err(|e| FetchError::Transport(e.to_string()))?;
            if html.len() > MAX_BYTES {
                return Err(FetchError::Transport("body exceeds 5 MB".to_string()));
            }

            let cfg = Config {
                text_mode: TextMode::Formatted,
                ..Default::default()
            };
            let mut readability = Readability::new(html.as_str(), Some(url), Some(cfg))
                .map_err(|e| FetchError::Parse(e.to_string()))?;
            let article = readability
                .parse()
                .map_err(|e| FetchError::Empty(e.to_string()))?;
            let title = article.title.trim();
            let body = article.text_content.trim();
            if body.is_empty() {
                return Err(FetchError::Empty("no readable content".to_string()));
            }
            Ok(if title.is_empty() {
                body.to_string()
            } else {
                format!("# {title}\n\n{body}")
            })
        })
    }
}

// ── Firecrawl (cloud / self-host scrape) ──────────────────────────────────────

pub struct FirecrawlFetch {
    inner: FirecrawlBackend,
}

impl FirecrawlFetch {
    pub fn new(inner: FirecrawlBackend) -> Self {
        Self { inner }
    }

    pub fn from_env() -> Self {
        Self::new(FirecrawlBackend::from_env())
    }
}

impl FetchBackend for FirecrawlFetch {
    fn name(&self) -> &str {
        "firecrawl"
    }

    fn fetch<'a>(
        &'a self,
        url: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, FetchError>> + Send + 'a>> {
        Box::pin(async move { self.inner.scrape(url).await.map_err(FetchError::from) })
    }
}

// ── Browser (headless Chrome) — feature-gated ─────────────────────────────────

#[cfg(feature = "browser")]
pub struct BrowserFetch {
    pool: std::sync::Arc<crate::browser::BrowserPool>,
}

#[cfg(feature = "browser")]
impl BrowserFetch {
    pub fn new(pool: std::sync::Arc<crate::browser::BrowserPool>) -> Self {
        Self { pool }
    }
}

#[cfg(feature = "browser")]
impl FetchBackend for BrowserFetch {
    fn name(&self) -> &str {
        "browser"
    }

    fn fetch<'a>(
        &'a self,
        url: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, FetchError>> + Send + 'a>> {
        let pool = std::sync::Arc::clone(&self.pool);
        let url = url.to_string();
        Box::pin(async move { pool.fetch_content(url).await.map_err(FetchError::Transport) })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn article_html() -> String {
        let para = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod \
            tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam.";
        format!(
            "<html><head><title>T</title></head><body><nav>NAVBOILER</nav>\
             <article><h1>H</h1><p>{para}</p><p>{para}</p><p>{para}</p></article></body></html>"
        )
    }

    #[tokio::test]
    async fn native_extracts_readable_text() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(200).set_body_string(article_html()))
            .mount(&server)
            .await;
        let out = NativeFetch::new()
            .fetch(&format!("{}/a", server.uri()))
            .await
            .unwrap();
        assert!(out.contains("Lorem ipsum"));
        assert!(!out.contains("NAVBOILER"));
    }

    #[tokio::test]
    async fn native_http_error_is_err() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let err = NativeFetch::new()
            .fetch(&format!("{}/x", server.uri()))
            .await
            .unwrap_err();
        assert!(matches!(err, FetchError::Transport(_)));
    }
}
