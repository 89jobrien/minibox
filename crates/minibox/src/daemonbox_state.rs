//! Minimal container state accessor trait used by adapters that need
//! to look up running container PIDs without depending on daemonbox.
//!
//! `DaemonState` in daemonbox implements this trait, breaking what would
//! otherwise be a circular dependency (minibox ← daemonbox ← minibox).

use std::sync::Arc;

/// Minimal state accessor needed by exec/commit adapters.
///
/// Implemented by `DaemonState` in the daemonbox crate.
#[async_trait::async_trait]
pub trait ContainerStateAccess: Send + Sync {
    /// Return the host-namespace PID of a running container's init process.
    async fn get_container_pid(&self, container_id: &str) -> anyhow::Result<u32>;

    /// Return the path to the container's overlay upper (writable) layer.
    async fn get_overlay_upper(&self, container_id: &str) -> anyhow::Result<std::path::PathBuf>;

    /// Return the image reference the container was started from.
    async fn get_source_image_ref(&self, container_id: &str) -> anyhow::Result<String>;
}

/// Shared handle to a [`ContainerStateAccess`] implementation.
pub type StateHandle = Arc<dyn ContainerStateAccess>;
