//! Host network adapter — container shares the host network namespace.
//!
//! Used when `NetworkMode::Host` is selected. The container process inherits
//! the host's network stack without any isolation.

use anyhow::Result;
use async_trait::async_trait;
use minibox_core::adapt;
use minibox_core::domain::{NetworkConfig, NetworkProvider, NetworkStats};

/// Network adapter that provides host networking mode.
///
/// In host mode the container shares the host's network namespace. `setup`
/// logs the intent and returns the sentinel string `"host"`. `attach`,
/// `cleanup`, and `stats` are no-ops because there is no separate namespace
/// to manage.
#[derive(Debug, Clone)]
pub struct HostNetwork;

impl HostNetwork {
    /// Create a new host network adapter.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NetworkProvider for HostNetwork {
    /// Log network setup and return `"host"` to signal host-namespace mode.
    async fn setup(&self, container_id: &str, _config: &NetworkConfig) -> Result<String> {
        tracing::info!(
            container_id = container_id,
            "network: host mode — container will share host network namespace"
        );
        Ok("host".to_string())
    }

    /// No-op — container is already in the host namespace, no attachment needed.
    async fn attach(&self, _container_id: &str, _pid: u32) -> Result<()> {
        Ok(())
    }

    /// No-op — no isolated namespace was created, so nothing to clean up.
    async fn cleanup(&self, _container_id: &str) -> Result<()> {
        Ok(())
    }

    /// Return default (all-zero) network statistics.
    ///
    /// Host-mode network statistics are not tracked at the container level.
    async fn stats(&self, _container_id: &str) -> Result<NetworkStats> {
        Ok(NetworkStats::default())
    }
}

adapt!(HostNetwork);

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::domain::NetworkConfig;

    #[tokio::test]
    async fn host_network_setup_returns_host_sentinel() {
        let net = HostNetwork::new();
        let result = net.setup("container-1", &NetworkConfig::default()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "host");
    }

    #[tokio::test]
    async fn host_network_attach_succeeds() {
        let net = HostNetwork::new();
        let result = net.attach("container-1", 12345).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn host_network_cleanup_succeeds() {
        let net = HostNetwork::new();
        let result = net.cleanup("container-1").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn host_network_stats_returns_default() {
        let net = HostNetwork::new();
        let result = net.stats("container-1").await;
        assert!(result.is_ok());
        let stats = result.unwrap();
        assert_eq!(stats.rx_bytes, 0);
        assert_eq!(stats.tx_bytes, 0);
    }
}
