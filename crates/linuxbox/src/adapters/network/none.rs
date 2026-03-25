//! No-op network adapter — all methods are no-ops that return immediately.
//!
//! Used when container networking is disabled (e.g., `NetworkMode::None`).
//! Containers get an isolated network namespace but no connectivity.

use anyhow::Result;
use async_trait::async_trait;
use minibox_core::adapt;
use minibox_core::domain::{NetworkConfig, NetworkProvider, NetworkStats};

/// Network adapter that disables all networking.
///
/// All methods are no-ops: `setup` returns an empty netns path, `attach` and
/// `cleanup` do nothing, and `stats` returns zeroed counters. Used when
/// `NetworkMode::None` is selected.
#[derive(Debug, Clone)]
pub struct NoopNetwork;

impl NoopNetwork {
    /// Create a new no-op network adapter.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NetworkProvider for NoopNetwork {
    /// Return an empty string — no network namespace is created.
    async fn setup(&self, _container_id: &str, _config: &NetworkConfig) -> Result<String> {
        Ok(String::new())
    }

    /// No-op — container remains in its isolated network namespace.
    async fn attach(&self, _container_id: &str, _pid: u32) -> Result<()> {
        Ok(())
    }

    /// No-op — nothing was created, so nothing to clean up.
    async fn cleanup(&self, _container_id: &str) -> Result<()> {
        Ok(())
    }

    /// Return default (all-zero) network statistics.
    async fn stats(&self, _container_id: &str) -> Result<NetworkStats> {
        Ok(NetworkStats::default())
    }
}

adapt!(NoopNetwork);

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::domain::NetworkConfig;

    #[tokio::test]
    async fn noop_network_setup_returns_empty_string() {
        let net = NoopNetwork::new();
        let result = net.setup("container-1", &NetworkConfig::default()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "");
    }

    #[tokio::test]
    async fn noop_network_attach_succeeds() {
        let net = NoopNetwork::new();
        let result = net.attach("container-1", 12345).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn noop_network_cleanup_succeeds() {
        let net = NoopNetwork::new();
        let result = net.cleanup("container-1").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn noop_network_stats_returns_default() {
        let net = NoopNetwork::new();
        let result = net.stats("container-1").await;
        assert!(result.is_ok());
        let stats = result.unwrap();
        assert_eq!(stats.rx_bytes, 0);
        assert_eq!(stats.tx_bytes, 0);
    }
}
