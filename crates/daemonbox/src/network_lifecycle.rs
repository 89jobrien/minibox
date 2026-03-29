//! Lifecycle wrapper for [`NetworkProvider`] with consistent error handling.

use anyhow::Result;
use minibox_core::domain::{DynNetworkProvider, NetworkConfig};
use tracing::warn;

/// Thin lifecycle wrapper around a [`NetworkProvider`].
///
/// Provides consistent setup/attach/cleanup with best-effort cleanup
/// semantics (cleanup logs warn on error, never propagates).
///
/// `NetworkLifecycle` is `Clone` because `run_inner` constructs it before
/// a `tokio::task::spawn` closure that must call `attach`. The inner
/// `DynNetworkProvider` is `Arc<dyn NetworkProvider>`,
/// so cloning is a cheap `Arc` refcount increment.
#[derive(Clone)]
pub struct NetworkLifecycle {
    provider: DynNetworkProvider,
}

impl NetworkLifecycle {
    /// Wrap a provider.
    pub fn new(provider: DynNetworkProvider) -> Self {
        Self { provider }
    }

    /// Set up network namespace for a new container.
    ///
    /// Returns the namespace path (e.g., `/var/run/netns/container-abc123`).
    pub async fn setup(&self, container_id: &str, config: &NetworkConfig) -> Result<String> {
        self.provider.setup(container_id, config).await
    }

    /// Attach a running container process to its network namespace.
    pub async fn attach(&self, container_id: &str, pid: u32) -> Result<()> {
        self.provider.attach(container_id, pid).await
    }

    /// Tear down networking for a container.
    ///
    /// Best-effort: logs `warn!` on error and never propagates the failure.
    /// Callers should not depend on the outcome.
    pub async fn cleanup(&self, container_id: &str) {
        if let Err(e) = self.provider.cleanup(container_id).await {
            warn!(
                container_id = %container_id,
                error = %e,
                "network: cleanup failed"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mbx::adapters::mocks::MockNetwork;
    use std::sync::Arc;

    #[tokio::test]
    async fn setup_delegates_to_provider_and_tracks_count() {
        let mock = Arc::new(MockNetwork::new());
        let net = NetworkLifecycle::new(mock.clone());
        let config = NetworkConfig::default();

        let result = net.setup("ctr1", &config).await;

        assert!(result.is_ok(), "setup should succeed with mock");
        assert_eq!(mock.setup_count(), 1);
    }

    #[tokio::test]
    async fn setup_propagates_provider_error() {
        let mock = Arc::new(MockNetwork::new().with_setup_failure());
        let net = NetworkLifecycle::new(mock.clone());
        let config = NetworkConfig::default();

        let result = net.setup("ctr1", &config).await;

        assert!(
            result.is_err(),
            "setup failure must be propagated to caller"
        );
        assert_eq!(mock.setup_count(), 1);
    }

    #[tokio::test]
    async fn attach_delegates_to_provider() {
        let mock = Arc::new(MockNetwork::new());
        let net = NetworkLifecycle::new(mock.clone());

        let result = net.attach("ctr1", 1234).await;

        assert!(result.is_ok(), "attach should succeed with mock");
    }

    #[tokio::test]
    async fn cleanup_delegates_to_provider_and_tracks_count() {
        let mock = Arc::new(MockNetwork::new());
        let net = NetworkLifecycle::new(mock.clone());

        net.cleanup("ctr1").await;

        assert_eq!(mock.cleanup_count(), 1);
    }

    #[tokio::test]
    async fn cleanup_swallows_provider_error() {
        let mock = Arc::new(MockNetwork::new().with_cleanup_failure());
        let net = NetworkLifecycle::new(mock.clone());

        // Must not panic or return error — best-effort cleanup
        net.cleanup("ctr1").await;

        assert_eq!(mock.cleanup_count(), 1, "cleanup must still be called once");
    }

    #[tokio::test]
    async fn lifecycle_clone_shares_provider() {
        let mock = Arc::new(MockNetwork::new());
        let net = NetworkLifecycle::new(mock.clone());
        let net2 = net.clone();

        let config = NetworkConfig::default();
        net.setup("ctr1", &config).await.unwrap();
        net2.setup("ctr2", &config).await.unwrap();

        // Both clones share the same provider Arc
        assert_eq!(mock.setup_count(), 2);
    }
}
