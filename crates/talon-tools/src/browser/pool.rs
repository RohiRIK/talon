//! `BrowserPool` — a lazily-launched, reused headless Chrome instance.
//!
//! One `Browser` is created on first use and reused across calls (each call
//! gets its own tab). A semaphore bounds the number of concurrent tabs. The
//! synchronous `headless_chrome` API is always driven inside `spawn_blocking`
//! so it never blocks the async runtime.

use std::sync::Arc;

use headless_chrome::Browser;
use tokio::sync::{Mutex, Semaphore};

pub struct BrowserPool {
    browser: Mutex<Option<Arc<Browser>>>,
    tabs: Semaphore,
}

impl BrowserPool {
    /// `max_tabs` bounds concurrent navigations against the shared browser.
    pub fn new(max_tabs: usize) -> Self {
        Self {
            browser: Mutex::new(None),
            tabs: Semaphore::new(max_tabs),
        }
    }

    /// Get (launching on first use) the shared browser.
    async fn browser(&self) -> Result<Arc<Browser>, String> {
        let mut guard = self.browser.lock().await;
        if guard.is_none() {
            let launched = tokio::task::spawn_blocking(Browser::default)
                .await
                .map_err(|e| format!("browser launch task failed: {e}"))?
                .map_err(|e| format!("could not launch headless Chrome: {e}"))?;
            *guard = Some(Arc::new(launched));
        }
        match guard.as_ref() {
            Some(b) => Ok(Arc::clone(b)),
            None => Err("browser unexpectedly absent after launch".to_string()),
        }
    }

    /// Navigate to `url` and return the rendered page HTML.
    pub async fn fetch_content(&self, url: String) -> Result<String, String> {
        let _permit = self
            .tabs
            .acquire()
            .await
            .map_err(|e| format!("browser pool closed: {e}"))?;
        let browser = self.browser().await?;

        tokio::task::spawn_blocking(move || -> Result<String, String> {
            let tab = browser.new_tab().map_err(|e| e.to_string())?;
            tab.navigate_to(&url).map_err(|e| e.to_string())?;
            tab.wait_until_navigated().map_err(|e| e.to_string())?;
            tab.get_content().map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| format!("navigation task failed: {e}"))?
    }
}

impl Default for BrowserPool {
    fn default() -> Self {
        Self::new(4)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn pool_constructs() {
        let _pool = BrowserPool::new(2);
        let _default = BrowserPool::default();
    }

    /// Real navigation — requires a Chrome/Chromium binary, so it is ignored by
    /// default and only run explicitly with `--ignored`.
    #[tokio::test]
    #[ignore = "requires a local Chrome binary"]
    async fn fetches_real_page() {
        let pool = BrowserPool::new(2);
        let html = pool
            .fetch_content("https://example.com".to_string())
            .await
            .expect("fetch");
        assert!(html.to_lowercase().contains("example domain"));
    }
}
