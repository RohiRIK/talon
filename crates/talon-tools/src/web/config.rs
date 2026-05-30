//! `[tools.web]` config and backend-chain assembly (Phase 5.1, task 5.14).
//!
//! Reads the web section of `~/.talon/config.toml` and builds the ordered
//! `SearchBackend` / `FetchBackend` chains. Missing/invalid config falls back
//! to the shipped defaults (Brave→DDG search, native-only fetch). API keys are
//! resolved from the environment by each backend's `from_env`.

use std::path::Path;

use serde::Deserialize;

use crate::web::backend::{BraveBackend, DdgBackend, SearchBackend};
use crate::web::fetch::{FetchBackend, FirecrawlFetch, NativeFetch};
use crate::web::firecrawl::FirecrawlBackend;
use crate::web::searxng::SearxngBackend;

fn default_search_backends() -> Vec<String> {
    vec!["brave".to_string(), "ddg".to_string()]
}

fn default_fetch_backends() -> Vec<String> {
    vec!["native".to_string()]
}

/// One backend's endpoint override (`[tools.web.<name>] url = "…"`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EndpointCfg {
    pub url: Option<String>,
}

/// The `[tools.web]` table.
#[derive(Debug, Clone, Deserialize)]
pub struct WebConfig {
    #[serde(default = "default_search_backends")]
    pub search_backends: Vec<String>,
    #[serde(default = "default_fetch_backends")]
    pub fetch_backends: Vec<String>,
    #[serde(default)]
    pub searxng: EndpointCfg,
    #[serde(default)]
    pub firecrawl: EndpointCfg,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            search_backends: default_search_backends(),
            fetch_backends: default_fetch_backends(),
            searxng: EndpointCfg::default(),
            firecrawl: EndpointCfg::default(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct ToolsSection {
    #[serde(default)]
    web: WebConfig,
}

#[derive(Debug, Default, Deserialize)]
struct RootConfig {
    #[serde(default)]
    tools: ToolsSection,
}

impl WebConfig {
    /// Parse `[tools.web]` out of a full config.toml string. Unknown sections
    /// are ignored; absent ones use defaults.
    pub fn parse(toml_str: &str) -> Self {
        match toml::from_str::<RootConfig>(toml_str) {
            Ok(root) => root.tools.web,
            Err(e) => {
                tracing::warn!("invalid [tools.web] config, using defaults: {e}");
                Self::default()
            }
        }
    }

    /// Load from a config file; a missing file yields defaults.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::parse(&s),
            Err(_) => Self::default(),
        }
    }

    /// Build the ordered search chain from `search_backends`.
    pub fn build_search_chain(&self) -> Vec<Box<dyn SearchBackend>> {
        let mut chain: Vec<Box<dyn SearchBackend>> = Vec::new();
        for name in &self.search_backends {
            match name.as_str() {
                "brave" => chain.push(Box::new(BraveBackend::from_env())),
                "ddg" | "duckduckgo" => chain.push(Box::new(DdgBackend::default_base())),
                "searxng" => match self.searxng.url.clone() {
                    Some(url) => chain.push(Box::new(SearxngBackend::new(url))),
                    None => {
                        tracing::warn!("searxng backend configured but no [tools.web.searxng] url")
                    }
                },
                "firecrawl" => chain.push(Box::new(firecrawl_from_cfg(&self.firecrawl))),
                other => tracing::warn!("unknown search backend '{other}'"),
            }
        }
        if chain.is_empty() {
            chain.push(Box::new(BraveBackend::from_env()));
            chain.push(Box::new(DdgBackend::default_base()));
        }
        chain
    }

    /// Build the ordered fetch chain from `fetch_backends`.
    pub fn build_fetch_chain(&self) -> Vec<Box<dyn FetchBackend>> {
        let mut chain: Vec<Box<dyn FetchBackend>> = Vec::new();
        for name in &self.fetch_backends {
            match name.as_str() {
                "native" => chain.push(Box::new(NativeFetch::new())),
                "firecrawl" => chain.push(Box::new(FirecrawlFetch::new(firecrawl_from_cfg(
                    &self.firecrawl,
                )))),
                "browser" => push_browser_fetch(&mut chain),
                other => tracing::warn!("unknown fetch backend '{other}'"),
            }
        }
        if chain.is_empty() {
            chain.push(Box::new(NativeFetch::new()));
        }
        chain
    }
}

/// Firecrawl backend honoring an optional config URL (else env / cloud).
fn firecrawl_from_cfg(cfg: &EndpointCfg) -> FirecrawlBackend {
    match &cfg.url {
        Some(url) => FirecrawlBackend::new(
            std::env::var("FIRECRAWL_API_KEY")
                .ok()
                .filter(|k| !k.is_empty()),
            url.clone(),
        ),
        None => FirecrawlBackend::from_env(),
    }
}

#[cfg(feature = "browser")]
fn push_browser_fetch(chain: &mut Vec<Box<dyn FetchBackend>>) {
    use crate::browser::BrowserPool;
    use crate::web::fetch::BrowserFetch;
    chain.push(Box::new(BrowserFetch::new(std::sync::Arc::new(
        BrowserPool::default(),
    ))));
}

#[cfg(not(feature = "browser"))]
fn push_browser_fetch(_chain: &mut Vec<Box<dyn FetchBackend>>) {
    tracing::warn!(
        "browser fetch backend requested but binary built without the 'browser' feature"
    );
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_uses_defaults() {
        let cfg = WebConfig::load(Path::new("/no/such/config.toml"));
        assert_eq!(cfg.search_backends, vec!["brave", "ddg"]);
        assert_eq!(cfg.fetch_backends, vec!["native"]);
    }

    #[test]
    fn parses_tools_web_section() {
        let toml = r#"
            [some.other.section]
            ignored = true

            [tools.web]
            search_backends = ["searxng", "brave", "ddg"]
            fetch_backends = ["native", "firecrawl"]

            [tools.web.searxng]
            url = "http://localhost:8080"
        "#;
        let cfg = WebConfig::parse(toml);
        assert_eq!(cfg.search_backends, vec!["searxng", "brave", "ddg"]);
        assert_eq!(cfg.searxng.url.as_deref(), Some("http://localhost:8080"));
    }

    #[test]
    fn search_chain_reflects_order_and_skips_unconfigured() {
        let cfg = WebConfig::parse(
            r#"
            [tools.web]
            search_backends = ["searxng", "ddg"]
            "#,
        );
        // searxng has no url → skipped; ddg remains.
        let chain = cfg.build_search_chain();
        let names: Vec<&str> = chain.iter().map(|b| b.name()).collect();
        assert_eq!(names, vec!["duckduckgo"]);
    }

    #[test]
    fn fetch_chain_defaults_to_native() {
        let names: Vec<String> = WebConfig::default()
            .build_fetch_chain()
            .iter()
            .map(|b| b.name().to_string())
            .collect();
        assert_eq!(names, vec!["native"]);
    }

    #[test]
    fn empty_search_list_falls_back_to_defaults() {
        let cfg = WebConfig::parse(
            r#"
            [tools.web]
            search_backends = []
            "#,
        );
        assert_eq!(cfg.build_search_chain().len(), 2);
    }
}
