# Web Search & Extract Tools

> **Status:** ✅ Complete
> **Category:** Core Features
> **Last corrected:** dogfood pass 3

---

## 1. Overview

Two read-only network tools cover web access:

| Tool | Description |
|------|-------------|
| `web_search` | Keyword search returning titles, URLs, descriptions |
| `web_extract` | Extract full markdown content from URLs (pages + PDFs) |

Both are pure `reqwest` HTTP calls — no [headless browser](32_Browser_Tool.md) needed.
Use `browser_navigate` only when JavaScript rendering is required.

---

## 2. WebSearch Tool

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WebSearchParams {
    /// Search query. Supports operators: site:, filetype:, intitle:, -term, "exact phrase"
    pub query: String,
    /// Maximum results (default: 5, max: 100)
    #[serde(default = "default_search_limit")]
    #[schemars(range(min = 1, max = 100))]
    pub limit: usize,
}

fn default_search_limit() -> usize { 5 }

pub struct WebSearchTool {
    backends: Vec<Arc<dyn SearchBackend>>,
    client: reqwest::Client,
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str { "web_search" }
    fn approval_level(&self) -> ApprovalLevel { ApprovalLevel::Confirmation }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let p: WebSearchParams = serde_json::from_value(args)?;

        // Try backends in priority order
        for backend in &self.backends {
            match backend.search(&p.query, p.limit).await {
                Ok(results) => {
                    return Ok(ToolResult::text(format_search_results(&results)));
                }
                Err(e) => {
                    tracing::warn!(backend = backend.name(), error = %e, "search backend failed");
                }
            }
        }

        Err(ToolError::AllBackendsFailed)
    }
}

fn format_search_results(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return "No results found.".to_string();
    }

    results.iter().enumerate().map(|(i, r)| {
        format!("{}. **{}**\n   {}\n   {}", i + 1, r.title, r.url, r.description)
    }).collect::<Vec<_>>().join("\n\n")
}
```

---

## 3. Search Backends

```rust
#[async_trait]
pub trait SearchBackend: Send + Sync {
    fn name(&self) -> &str;
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, SearchError>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub description: String,
}
```

### Brave Search Backend

```rust
pub struct BraveSearchBackend {
    api_key: SecretString,
    client: reqwest::Client,
}

#[async_trait]
impl SearchBackend for BraveSearchBackend {
    fn name(&self) -> &str { "brave" }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, SearchError> {
        let resp: serde_json::Value = self.client
            .get("https://api.search.brave.com/res/v1/web/search")
            .header("X-Subscription-Token", self.api_key.expose_secret())
            .header("Accept", "application/json")
            .query(&[("q", query), ("count", &limit.to_string())])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let results = resp["web"]["results"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|r| SearchResult {
                title: r["title"].as_str().unwrap_or("").to_string(),
                url: r["url"].as_str().unwrap_or("").to_string(),
                description: r["description"].as_str().unwrap_or("").to_string(),
            })
            .collect();

        Ok(results)
    }
}
```

### SearXNG Backend (self-hosted)

```rust
pub struct SearxngBackend {
    base_url: String,
    client: reqwest::Client,
}

#[async_trait]
impl SearchBackend for SearxngBackend {
    fn name(&self) -> &str { "searxng" }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, SearchError> {
        let resp: SearxngResponse = self.client
            .get(format!("{}/search", self.base_url))
            .query(&[
                ("q", query),
                ("format", "json"),
                ("pageno", "1"),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        Ok(resp.results.into_iter().take(limit).map(|r| SearchResult {
            title: r.title,
            url: r.url,
            description: r.content.unwrap_or_default(),
        }).collect())
    }
}
```

---

## 4. WebExtract Tool

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WebExtractParams {
    /// URLs to extract content from (max 5)
    #[schemars(length(max = 5))]
    pub urls: Vec<String>,
}

pub struct WebExtractTool {
    client: reqwest::Client,
    pdf_extractor: Option<Arc<dyn PdfExtractor>>,
}

#[async_trait]
impl Tool for WebExtractTool {
    fn name(&self) -> &str { "web_extract" }
    fn approval_level(&self) -> ApprovalLevel { ApprovalLevel::Confirmation }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let p: WebExtractParams = serde_json::from_value(args)?;

        if p.urls.len() > 5 {
            return Err(ToolError::InvalidParams("max 5 URLs per call".into()));
        }

        // Fetch all in parallel
        let futures: Vec<_> = p.urls.iter()
            .map(|url| self.extract_url(url))
            .collect();

        let results = futures::future::join_all(futures).await;

        let output = results.iter().zip(&p.urls).map(|(result, url)| {
            match result {
                Ok(content) => format!("## {url}\n\n{content}"),
                Err(e) => format!("## {url}\n\nError: {e}"),
            }
        }).collect::<Vec<_>>().join("\n\n---\n\n");

        Ok(ToolResult::text(output))
    }
}

impl WebExtractTool {
    async fn extract_url(&self, url: &str) -> Result<String, ExtractError> {
        // Detect PDF
        if url.ends_with(".pdf") || url.contains("/pdf/") {
            return self.extract_pdf(url).await;
        }

        let resp = tokio::time::timeout(
            Duration::from_secs(15),
            self.client
                .get(url)
                .header("User-Agent", "Mozilla/5.0 (compatible; talon-bot/1.0)")
                .send(),
        )
        .await
        .map_err(|_| ExtractError::Timeout)?
        .map_err(ExtractError::Http)?
        .error_for_status()
        .map_err(ExtractError::Http)?;

        // Check content type
        let ct = resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if ct.contains("application/pdf") {
            return self.extract_pdf(url).await;
        }

        let html = resp.text().await.map_err(ExtractError::Http)?;
        self.html_to_markdown(&html)
    }

    fn html_to_markdown(&self, html: &str) -> Result<String, ExtractError> {
        use scraper::{Html, Selector};

        let document = Html::parse_document(html);

        // Extract title
        let title_sel = Selector::parse("title").unwrap();
        let title = document.select(&title_sel)
            .next()
            .map(|e| e.text().collect::<String>())
            .unwrap_or_default();

        // Extract main content (prefer article/main over body)
        let content_sel = Selector::parse("article, main, .content, #content, body").unwrap();
        let content_el = document.select(&content_sel).next();

        let text = if let Some(el) = content_el {
            extract_text_blocks(el)
        } else {
            String::new()
        };

        // Convert to basic markdown using htmd
        let markdown = htmd::convert(&text).unwrap_or(text);

        // Truncate if too long
        let truncated = if markdown.len() > 30_000 {
            format!("{}\n\n... [truncated]", &markdown[..30_000])
        } else {
            markdown
        };

        Ok(if title.is_empty() {
            truncated
        } else {
            format!("# {title}\n\n{truncated}")
        })
    }

    async fn extract_pdf(&self, url: &str) -> Result<String, ExtractError> {
        let bytes = self.client.get(url).send().await?.bytes().await?;

        // Use pdf-extract crate (wraps poppler)
        let text = pdf_extract::extract_text_from_mem(&bytes)
            .map_err(|e| ExtractError::Pdf(e.to_string()))?;

        Ok(format!("[PDF content]\n\n{}", &text[..text.len().min(30_000)]))
    }
}
```

---

## 5. Rate Limiting & Caching

```rust
pub struct RateLimitedClient {
    client: reqwest::Client,
    limiter: Arc<RateLimiter<String>>,
    cache: Arc<DashMap<String, CachedResponse>>,
    cache_ttl: Duration,
}

impl RateLimitedClient {
    pub async fn get(&self, url: &str) -> Result<String, ExtractError> {
        // Cache hit
        if let Some(cached) = self.cache.get(url) {
            if cached.expires_at > Instant::now() {
                return Ok(cached.content.clone());
            }
        }

        // Rate limit by domain
        let domain = extract_domain(url);
        self.limiter.check_key(&domain)?;  // governor crate

        let content = /* fetch */ todo!();

        // Cache response
        self.cache.insert(url.to_string(), CachedResponse {
            content: content.clone(),
            expires_at: Instant::now() + self.cache_ttl,
        });

        Ok(content)
    }
}
```

> **Last corrected:** dogfood pass 4
---

## Related Documents

### Depends On
- [Tool System Architecture](../02_Architecture/16_Tool_System_Architecture.md)

### See Also
- [Browser Tool](32_Browser_Tool.md)
- [MCP Client Tool](36_MCP_Client_Tool.md)

