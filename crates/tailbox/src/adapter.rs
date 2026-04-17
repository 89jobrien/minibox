use anyhow::Result;
use async_trait::async_trait;
use minibox_core::domain::{AsAny, NetworkConfig, NetworkProvider, NetworkStats};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config::TailnetConfig;

/// Tailscale-rs network adapter for minibox containers.
///
/// **Stability warning**: This adapter wraps `tailscale-rs` v0.2, which is pre-1.0,
/// contains unaudited cryptography, and has no backwards-compatibility guarantees.
/// Do not use in production until tailscale-rs completes a third-party security audit.
/// See: <https://github.com/tailscale/tailscale-rs#caveats>
pub struct TailnetNetwork {
    #[allow(dead_code)]
    config: TailnetConfig,
    /// Per-container devices (PerContainer mode). Keyed by container_id.
    devices: Arc<Mutex<HashMap<String, tailscale::Device>>>,
    /// Shared gateway device (Gateway mode). Lazily initialised on first use.
    #[allow(dead_code)]
    gateway_device: Arc<Mutex<Option<tailscale::Device>>>,
}

impl TailnetNetwork {
    /// Create a new `TailnetNetwork` adapter.
    pub async fn new(config: TailnetConfig) -> Result<Self> {
        Ok(Self {
            config,
            devices: Arc::new(Mutex::new(HashMap::new())),
            gateway_device: Arc::new(Mutex::new(None)),
        })
    }
}

impl AsAny for TailnetNetwork {
    fn as_any(&self) -> &dyn ::std::any::Any {
        self
    }
}

#[async_trait]
impl NetworkProvider for TailnetNetwork {
    async fn setup(&self, container_id: &str, config: &NetworkConfig) -> Result<String> {
        // Full implementation in Task 4.
        let _ = (container_id, config);
        anyhow::bail!("tailnet: setup not yet implemented")
    }

    async fn attach(&self, _container_id: &str, _pid: u32) -> Result<()> {
        // tailscale-rs devices are not pid-namespace-based; attach is a no-op.
        Ok(())
    }

    async fn cleanup(&self, container_id: &str) -> Result<()> {
        // Remove any per-container device (no-op if not present).
        self.devices.lock().await.remove(container_id);

        // Remove context file (best-effort).
        let ctx_path = std::path::Path::new("/run/minibox/net")
            .join(format!("{container_id}.json"));
        if let Err(e) = std::fs::remove_file(&ctx_path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                container_id = container_id,
                error = %e,
                "tailnet: could not remove net context file"
            );
        }

        Ok(())
    }

    async fn stats(&self, _container_id: &str) -> Result<NetworkStats> {
        // tailscale-rs v0.2 has no stats API.
        Ok(NetworkStats::default())
    }
}
