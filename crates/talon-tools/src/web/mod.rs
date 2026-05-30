//! Web tools (Phase 5): fetch + readable extraction, search.

pub mod backend;
pub mod config;
pub mod extract;
pub mod fetch;
pub mod firecrawl;
pub mod search;
pub mod searxng;

pub use backend::{BraveBackend, DdgBackend, SearchBackend, SearchError, SearchResult};
pub use config::WebConfig;
pub use extract::WebExtractTool;
pub use fetch::{FetchBackend, FetchError, FirecrawlFetch, NativeFetch};
pub use firecrawl::FirecrawlBackend;
pub use search::WebSearchTool;
pub use searxng::SearxngBackend;
