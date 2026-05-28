use std::collections::HashMap;
use std::sync::Arc;

use crate::Gateway;

pub type ChannelId = String;

/// Registry mapping channel IDs to gateway instances.
pub struct GatewayRegistry {
    gateways: HashMap<ChannelId, Arc<dyn Gateway>>,
}

impl GatewayRegistry {
    pub fn new() -> Self {
        Self {
            gateways: HashMap::new(),
        }
    }

    pub fn register(&mut self, id: impl Into<ChannelId>, gateway: Arc<dyn Gateway>) {
        self.gateways.insert(id.into(), gateway);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Gateway>> {
        self.gateways.get(id).cloned()
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.gateways.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.gateways.len()
    }

    pub fn is_empty(&self) -> bool {
        self.gateways.is_empty()
    }
}

impl Default for GatewayRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use crate::{GatewayError, RenderMode};

    struct DummyGateway;
    impl Gateway for DummyGateway {
        fn name(&self) -> &str { "dummy" }
        fn render_mode(&self) -> RenderMode { RenderMode::Plain }
        fn run(&self) -> Pin<Box<dyn Future<Output = Result<(), GatewayError>> + Send + '_>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[test]
    fn registry_register_and_get() {
        let mut reg = GatewayRegistry::new();
        reg.register("cli", Arc::new(DummyGateway));
        assert!(reg.get("cli").is_some());
        assert!(reg.get("telegram").is_none());
    }

    #[test]
    fn registry_len() {
        let mut reg = GatewayRegistry::new();
        assert_eq!(reg.len(), 0);
        reg.register("cli", Arc::new(DummyGateway));
        reg.register("http", Arc::new(DummyGateway));
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn registry_ids() {
        let mut reg = GatewayRegistry::new();
        reg.register("tui", Arc::new(DummyGateway));
        let ids: Vec<&str> = reg.ids().collect();
        assert!(ids.contains(&"tui"));
    }

    #[test]
    fn registry_default_is_empty() {
        let reg = GatewayRegistry::default();
        assert!(reg.is_empty());
    }
}
