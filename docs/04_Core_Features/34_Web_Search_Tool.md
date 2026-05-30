# Web Search & Extract Tools

> **Status:** Phase 5 shipped (Brave→DDG search, native Rust extract, headless browser). Multi-backend chain (SearXNG, Firecrawl) is planned — see §6.
> **Category:** Core Features
> **Last corrected:** 2026-05-29 — reconciled with shipped code + multi-backend design.

---

## 1. Overview

Talon separates two distinct problems that are often conflated:

| Problem | Tool | Can it be pure-Rust / self-hosted? |
|---------|------|------------------------------------|
| **Search** — query → ranked list of URLs | `web_search` | **No.** Always needs an external *index*. SearXNG (self-host) is the closest. |
| **Fetch/extract** — URL → readable content | `web_extract` | **Yes.** `reqwest` + `dom_smoothie` (readability). Zero external service. |
| **Render** — JS-heavy / SPA pages → content | `browser_open` | **Yes.** `headless_chrome` (own browser). External (Firecrawl) only as a convenience. |

**Design principle:** the *fetch floor is always our own Rust*. External services (Brave, Firecrawl) are optional conveniences or fallbacks, never hard requirements. The only thing we can't self-build is a web **index**, so search degrades through a configurable backend chain.

All three tools route through the existing `Tool` trait (`execute -> Pin<Box<dyn Future>>`, no `async-trait` — ADR 0007) and the `ApprovalMembrane`.

---

## 2. Tool surface (as shipped, Phase 5)

| Tool | Approval | Backend (shipped) | Notes |
|------|----------|-------------------|-------|
| `web_search` | `Safe` | Brave API → DuckDuckGo (HTML scrape) fallback | `BRAVE_API_KEY` optional; keyless via DDG |
| `web_extract` | `Safe` | `reqwest` + `dom_smoothie` (Readability) | 5 MB cap, boilerplate stripped, no JS |
| `browser_open` | `NeedsApproval` | `headless_chrome` via `BrowserPool` | `feature = "browser"`, off by default |

Real trait shape (do not regress to `async fn`/`Result<ToolResult>`):

```rust
fn execute(&self, args: Value, ctx: ToolContext)
    -> Pin<Box<dyn Future<Output = ToolResult> + Send + '_>>;
// errors are returned as ToolResult::err(..), never propagated
```

---

## 3. Search: the backend chain

How other agents do it (verified against Open WebUI's source): **one module per backend, all normalizing to a common result type, selected by config.** Talon follows the same shape with an ordered *fallback* chain.

```rust
pub trait SearchBackend: Send + Sync {
    fn name(&self) -> &str;
    fn search(&self, query: &str, count: u32)
        -> Pin<Box<dyn Future<Output = Result<Vec<SearchResult>, SearchError>> + Send + '_>>;
}

pub struct SearchResult { pub title: String, pub url: String, pub snippet: String }
```

`WebSearchTool` tries backends in configured order; on error **or empty results**, it falls through to the next. Each backend:

| Backend | Endpoint / mechanism | Auth | Self-host |
|---------|----------------------|------|-----------|
| **Brave** | `GET api.search.brave.com/res/v1/web/search?q=&count=` | `X-Subscription-Token` header | cloud only (free tier 1 req/s → 429 retry) |
| **SearXNG** | `GET {instance}/search?q=&format=json` → `results[]` | none | **yes** — you run it; meta-searches Google/Bing/DDG; `format=json` must be enabled |
| **Firecrawl** | `POST {base}/v1/search` | `Bearer` | **cloud or self-host** (`base` overridable) |
| **DuckDuckGo** | scrape `html.duckduckgo.com/html/?q=` (`result__a` anchors) | none | n/a — fragile, last-resort fallback |

> Reality check: there is **no pure-Rust web search** — every option above borrows someone's index. SearXNG self-hosted is the only "run it yourself" search.

---

## 4. Fetch / extract: native first, escalate only when needed

```rust
pub trait FetchBackend: Send + Sync {
    fn name(&self) -> &str;
    fn fetch(&self, url: &str)
        -> Pin<Box<dyn Future<Output = Result<String, FetchError>> + Send + '_>>;
}
```

Ordered chain (configurable); **native is the default floor**:

| Backend | How | When |
|---------|-----|------|
| **native** (shipped) | `reqwest` GET → `dom_smoothie` Readability → text | default; all static HTML |
| **browser** (shipped) | `headless_chrome` renders JS, returns DOM HTML | SPA / JS-required pages; `NeedsApproval` |
| **firecrawl** (planned) | `POST {base}/v1/scrape {formats:["markdown"]}` → markdown | hard pages / anti-bot, or when you don't want local Chrome |

Escalation policy: use `native` first; fall to `browser`/`firecrawl` only if native returns empty/garbage, or the caller/config explicitly requests rendering.

---

## 5. Configuration (`~/.talon/config.toml`)

```toml
[tools.web]
# Search backends tried in order; first non-empty wins.
search_backends = ["brave", "searxng", "ddg"]   # add "firecrawl" if licensed
# Fetch backends tried in order; native is the floor.
fetch_backends  = ["native", "browser"]          # add "firecrawl" optionally

[tools.web.brave]
# api_key read from BRAVE_API_KEY env / OS keychain if omitted

[tools.web.searxng]
url = "http://localhost:8080"                    # your self-hosted instance

[tools.web.firecrawl]
url     = "https://api.firecrawl.dev"            # or your self-hosted base
# api_key from FIRECRAWL_API_KEY env / keychain
```

Timeouts (task 5.9): web = 30s, browser = 60s (`TimeoutWrapper` at registration).

---

## 6. Implementation status & next steps

**Shipped (Phase 5 + 5.1):**
- `SearchBackend` chain (`web/backend.rs`): `BraveBackend`, `DdgBackend`, `SearxngBackend` (`web/searxng.rs`), `FirecrawlBackend` (`web/firecrawl.rs`). `WebSearchTool` tries them in configured order; first non-empty wins.
- `FetchBackend` chain (`web/fetch.rs`): `NativeFetch` (reqwest + dom_smoothie, the floor), `BrowserFetch` (`feature="browser"`), `FirecrawlFetch`. `WebExtractTool` escalates native → next on empty/err.
- `[tools.web]` config (`web/config.rs`): `search_backends` / `fetch_backends` order + `[tools.web.{searxng,firecrawl}]` URLs; assembled in `build_gateway_context`. Keys from env (Brave `BRAVE_API_KEY`, Firecrawl `FIRECRAWL_API_KEY`/`FIRECRAWL_API_URL`).
- `browser_open` tool (headless Chrome, feature-gated), per-class timeouts.

Native Rust fetch is the dependency-free default; every external service is opt-in via config. **Next:** resolve keys via OS keychain (not just env); optional `firecrawl` in default search order once a key-presence probe gates it.

---

## 7. Prior art (verified against source, 2026-05-29)

| Agent | Search | Fetch / extract content | Notes |
|-------|--------|-------------------------|-------|
| **Open WebUI** | one module per engine (brave/searxng/ddg/firecrawl/tavily…), normalized to `SearchResult{link,title,snippet}`, **selected by config** | via the search engines / Firecrawl | the canonical multi-backend pattern |
| **Hermes** (`NousResearch/hermes-agent`) | `WebSearchProvider` ABC + registry; keyless built-ins = `ddgs` (DuckDuckGo scrape) and `searxng` (self-host); `brave_free` | **only via external providers** — Firecrawl, Tavily, Exa, Parallel, xai. **No native/own-code extractor.** | provider selected via `web.search_backend`/`web.extract_backend` in `config.yaml` |
| **Claude Code** | hosted server-side search (Anthropic `web_search`) | `WebFetch` (fetch → markdown → model) | no Brave/DDG |
| **Talon** | Brave → DDG (shipped); SearXNG/Firecrawl planned | **native Rust** `reqwest` + `dom_smoothie` (shipped) → `headless_chrome` → Firecrawl | native fetch is the floor — see §4 |

**Key takeaway:** for *fetch/extract*, Hermes (and Open WebUI) **delegate to external SaaS** (Firecrawl/Tavily/Exa) — they have no in-process page extractor. Talon's native `reqwest + dom_smoothie` path means web_extract works with **zero external service**, which those agents can't do. For *search*, everyone needs an external index; the only self-hosted option is SearXNG (which itself proxies Google/Bing/etc.) — there is no pure-local web search.

---

## Related Documents

### Depends On
- [Tool System Architecture](../02_Architecture/16_Tool_System_Architecture.md)

### See Also
- [Browser Tool](32_Browser_Tool.md)
- [MCP Client Tool](36_MCP_Client_Tool.md)
