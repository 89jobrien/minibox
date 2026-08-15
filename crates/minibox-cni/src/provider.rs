//! `NetworkProvider` adapter implementing minibox-core's port via CNI
//! plugin chains.

use crate::config::NetworkConfigList;
use anyhow::Context as _;
use async_trait::async_trait;
use minibox_core::domain::{AsAny, NetworkConfig, NetworkProvider, NetworkStats};
use std::any::Any;
use std::path::PathBuf;
use tracing::instrument;

/// Adapter implementing minibox-core's `NetworkProvider` port via CNI plugin chains.
///
/// Tracks the network namespace path used for each container's `attach()`
/// call so `cleanup()` (which only receives a `container_id`) can run the
/// matching `DEL` chain.
#[derive(Debug)]
pub struct CniNetworkProvider {
    /// Directories searched for plugin binaries, in order.
    pub cni_path: Vec<PathBuf>,
    /// Directory containing the `.conflist` network configuration.
    pub config_dir: PathBuf,
    netns_by_container: dashmap::DashMap<String, String>,
}

impl CniNetworkProvider {
    /// Create a new CNI-backed network provider.
    #[must_use]
    pub fn new(cni_path: Vec<PathBuf>, config_dir: PathBuf) -> Self {
        Self {
            cni_path,
            config_dir,
            netns_by_container: dashmap::DashMap::new(),
        }
    }

    fn conflist_path(&self) -> PathBuf {
        self.config_dir.join("10-minibox.conflist")
    }
}

impl AsAny for CniNetworkProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[async_trait]
impl NetworkProvider for CniNetworkProvider {
    #[instrument(skip(self, _config), fields(container_id = %container_id))]
    async fn setup(&self, container_id: &str, _config: &NetworkConfig) -> anyhow::Result<String> {
        // Namespace attach happens in `attach()` once the container PID is
        // known; `setup()` only validates the .conflist is present and
        // parseable up front, so misconfiguration fails fast before the
        // container process is even spawned.
        NetworkConfigList::from_file(&self.conflist_path()).context("loading CNI .conflist")?;
        Ok(container_id.to_string())
    }

    #[instrument(skip(self), fields(container_id = %container_id, pid = pid))]
    async fn attach(&self, container_id: &str, pid: u32) -> anyhow::Result<()> {
        let conflist =
            NetworkConfigList::from_file(&self.conflist_path()).context("loading CNI .conflist")?;
        let netns = format!("/proc/{pid}/ns/net");
        conflist
            .add(&self.cni_path, &netns, container_id, "eth0")
            .await
            .context("running CNI ADD chain")?;
        self.netns_by_container
            .insert(container_id.to_string(), netns);
        Ok(())
    }

    #[instrument(skip(self), fields(container_id = %container_id))]
    async fn cleanup(&self, container_id: &str) -> anyhow::Result<()> {
        let (_, netns) = self
            .netns_by_container
            .remove(container_id)
            .ok_or_else(|| {
                anyhow::anyhow!("no recorded network namespace for container {container_id}")
            })?;
        let conflist =
            NetworkConfigList::from_file(&self.conflist_path()).context("loading CNI .conflist")?;
        conflist
            .del(&self.cni_path, &netns, container_id, "eth0")
            .await
            .context("running CNI DEL chain")?;
        Ok(())
    }

    async fn stats(&self, _container_id: &str) -> anyhow::Result<NetworkStats> {
        anyhow::bail!("CniNetworkProvider does not yet implement stats collection")
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use minibox_core::domain::NetworkMode;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    fn write_conflist(dir: &std::path::Path) {
        let mut file =
            std::fs::File::create(dir.join("10-minibox.conflist")).expect("create conflist");
        file.write_all(
            br#"{"cniVersion":"1.0.0","name":"minibox0","plugins":[{"type":"fake-noop"}]}"#,
        )
        .expect("write conflist");
    }

    fn write_noop_plugin(dir: &std::path::Path) {
        let path = dir.join("fake-noop");
        std::fs::write(&path, "#!/bin/sh\nif [ \"$CNI_COMMAND\" = \"ADD\" ]; then\n  echo '{\"cniVersion\":\"1.0.0\"}'\nfi\nexit 0\n").expect("write plugin");
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
    }

    #[tokio::test]
    async fn attach_then_cleanup_uses_recorded_netns() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let bin_dir = tempfile::tempdir().expect("bin dir");
        write_conflist(config_dir.path());
        write_noop_plugin(bin_dir.path());

        let provider = CniNetworkProvider::new(
            vec![bin_dir.path().to_path_buf()],
            config_dir.path().to_path_buf(),
        );

        let config = minibox_core::domain::NetworkConfig {
            mode: NetworkMode::Bridge,
            ..Default::default()
        };
        provider
            .setup("container-1", &config)
            .await
            .expect("setup should succeed");
        provider
            .attach("container-1", 1)
            .await
            .expect("attach should succeed");
        provider
            .cleanup("container-1")
            .await
            .expect("cleanup should succeed");
    }

    #[tokio::test]
    async fn cleanup_without_prior_attach_returns_error() {
        let config_dir = tempfile::tempdir().expect("config dir");
        write_conflist(config_dir.path());
        let provider = CniNetworkProvider::new(vec![], config_dir.path().to_path_buf());

        let result = provider.cleanup("never-attached").await;
        assert!(result.is_err());
    }
}
