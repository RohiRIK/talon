//! Web tools (Phase 5): fetch + readable extraction, search.

pub mod backend;
pub mod extract;
pub mod search;
pub mod searxng;

pub use backend::{BraveBackend, DdgBackend, SearchBackend, SearchError, SearchResult};
pub use extract::WebExtractTool;
pub use search::WebSearchTool;
pub use searxng::SearxngBackend;
