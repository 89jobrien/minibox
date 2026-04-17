use anyhow::Result;
use async_trait::async_trait;
use minibox_core::domain::{AsAny, NetworkConfig, NetworkProvider, NetworkStats, TailnetMode};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::auth::resolve_auth_key;
use crate::config::TailnetConfig;
use crate::experiment::ensure_tsrs_experiment;

/// Tailscale-rs network adapter for minibox containers.
///
/// **Stability warning**: This adapter wraps `tailscale-rs` v0.2, which is pre-1.0,
/// contains unaudited cryptography, and has no backwards-compatibility guarantees.
/// Do not use in production until tailscale-rs completes a third-party security audit.
/// See: <https://github.com/tailscale/tailscale-rs#caveats>
pub struct TailnetNetwork {
    config: TailnetConfig,
    /// Per-container devices (PerContainer mode). Keyed by container_id.
    devices: Arc<Mutex<HashMap<String, tailscale::Device>>>,
    /// Shared gateway device (Gateway mode). Lazily initialised on first use.
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

    /// Path to the per-container key file.
    fn container_key_path(id: &str) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        PathBuf::from(home).join(".mbx").join("tailnet").join(format!("{id}.json"))
    }

    /// Path to the gateway key file.
    fn gateway_key_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        PathBuf::from(home).join(".mbx").join("tailnet").join("gateway.json")
    }

    /// Path to the net-context JSON written after setup.
    fn net_context_path(id: &str) -> PathBuf {
        PathBuf::from("/run/minibox/net").join(format!("{id}.json"))
    }

    /// Set up gateway mode: lazily initialise one shared device.
    ///
    /// Returns the context JSON string.
    async fn setup_gateway(&self, container_id: &str, auth_key: &str) -> Result<String> {
        let mut guard = self.gateway_device.lock().await;

        if guard.is_none() {
            let key_path = Self::gateway_key_path();
            // NOTE: tailscale-rs v0.2 load_key_file creates parent dirs automatically.
            let key_state =
                tailscale::load_key_file(&key_path, tailscale::BadFormatBehavior::Overwrite)
                    .await
                    .map_err(|e| anyhow::anyhow!("tailnet: load_key_file failed: {e}"))?;

            // NOTE: tailscale::Config has no auth_key field — auth key is the second
            // parameter to Device::new, not part of Config.
            let cfg = tailscale::Config {
                key_state,
                ..Default::default()
            };
            let device = tailscale::Device::new(&cfg, Some(auth_key.to_string()))
                .await
                .map_err(|e| anyhow::anyhow!("tailnet: Device::new failed: {e}"))?;

            *guard = Some(device);
        }

        // SAFETY: we just ensured it is Some above.
        let device = guard.as_ref().expect("gateway device must be Some");
        let tailnet_ip = device
            .ipv4_addr()
            .await
            .map_err(|e| anyhow::anyhow!("tailnet: ipv4_addr failed: {e}"))?;

        let ctx = serde_json::json!({
            "mode": "gateway",
            "tailnet_ip": tailnet_ip.to_string(),
            "container_id": container_id,
        });
        Ok(ctx.to_string())
    }

    /// Set up per-container mode: create a dedicated device for this container.
    ///
    /// Returns the context JSON string.
    async fn setup_per_container(&self, container_id: &str, auth_key: &str) -> Result<String> {
        let key_path = Self::container_key_path(container_id);
        // NOTE: tailscale-rs v0.2 load_key_file creates parent dirs automatically.
        let key_state =
            tailscale::load_key_file(&key_path, tailscale::BadFormatBehavior::Overwrite)
                .await
                .map_err(|e| anyhow::anyhow!("tailnet: load_key_file failed: {e}"))?;

        // NOTE: auth key passed as second arg to Device::new, not via Config.
        let cfg = tailscale::Config {
            key_state,
            ..Default::default()
        };
        let device = tailscale::Device::new(&cfg, Some(auth_key.to_string()))
            .await
            .map_err(|e| anyhow::anyhow!("tailnet: Device::new failed: {e}"))?;

        let tailnet_ip = device
            .ipv4_addr()
            .await
            .map_err(|e| anyhow::anyhow!("tailnet: ipv4_addr failed: {e}"))?;

        let tailnet_ip_str = tailnet_ip.to_string();

        tracing::info!(
            container_id = container_id,
            tailnet_ip = %tailnet_ip_str,
            "tailnet: per-container device joined"
        );

        self.devices
            .lock()
            .await
            .insert(container_id.to_string(), device);

        let ctx = serde_json::json!({
            "mode": "per_container",
            "tailnet_ip": tailnet_ip_str,
            "container_id": container_id,
        });
        Ok(ctx.to_string())
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
        ensure_tsrs_experiment();

        let auth_key = resolve_auth_key(config, &self.config.key_secret_name).await?;

        let ctx_json = match config.tailnet_mode {
            TailnetMode::Gateway => {
                self.setup_gateway(container_id, &auth_key).await?
            }
            TailnetMode::PerContainer => {
                self.setup_per_container(container_id, &auth_key).await?
            }
        };

        // Write context JSON to /run/minibox/net/{container_id}.json (best-effort).
        let ctx_path = Self::net_context_path(container_id);
        if let Some(parent) = ctx_path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::warn!(
                container_id = container_id,
                error = %e,
                "tailnet: could not create net context dir"
            );
        }
        if let Err(e) = std::fs::write(&ctx_path, &ctx_json) {
            tracing::warn!(
                container_id = container_id,
                error = %e,
                "tailnet: could not write net context file"
            );
        }

        tracing::info!(
            container_id = container_id,
            mode = ?config.tailnet_mode,
            "tailnet: network setup complete"
        );

        Ok(ctx_json)
    }

    async fn attach(&self, _container_id: &str, _pid: u32) -> Result<()> {
        // tailscale-rs devices are not pid-namespace-based; attach is a no-op.
        Ok(())
    }

    async fn cleanup(&self, container_id: &str) -> Result<()> {
        let removed = self.devices.lock().await.remove(container_id);

        if removed.is_some() {
            // Delete per-container key file (best-effort).
            let key_path = Self::container_key_path(container_id);
            if let Err(e) = std::fs::remove_file(&key_path) {
                tracing::warn!(
                    container_id = container_id,
                    error = %e,
                    "tailnet: could not remove container key file"
                );
            }
        }

        // Remove context file (best-effort).
        let ctx_path = Self::net_context_path(container_id);
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
