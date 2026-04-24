//! VZ adapter suite — four domain trait implementations that forward to the
//! in-VM miniboxd agent over vsock.
//!
//! Each adapter holds a shared reference to the running [`VzVm`] and
//! delegates every domain operation to the guest daemon by:
//!
//! 1. Building the appropriate [`DaemonRequest`] variant.
//! 2. Opening a vsock connection via [`connect_to_agent`].
//! 3. Sending the request through a [`VzProxy`] and collecting responses.
//! 4. Extracting the result from the terminal [`DaemonResponse`].
//!
//! The domain traits are synchronous (or `async_trait` async), so vsock calls
//! are driven via `tokio::runtime::Handle::current().block_on(…)` for the
//! synchronous ones, and awaited directly for the async ones.
//!
//! # Feature gate
//!
//! This module compiles on all platforms but the real [`VzVm`] / vsock
//! plumbing is only present on macOS with the `vz` feature.  On other
//! platforms every method returns an error immediately, keeping the stub
//! adapters fully type-correct.

use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use minibox_core::{
    domain::{
        AsAny, ChildInit, ContainerRuntime, ContainerSpawnConfig, FilesystemProvider,
        ImageMetadata, ImageRegistry, ResourceConfig, ResourceLimiter, RootfsLayout, RootfsSetup,
        RuntimeCapabilities, SpawnResult,
    },
    image::reference::ImageRef,
    protocol::{DaemonRequest, DaemonResponse},
};

use crate::vz::{VzProxy, vm::VzVm, vsock::connect_to_agent};

// ---------------------------------------------------------------------------
// Helper: send one request to the in-VM agent and return all responses.
// ---------------------------------------------------------------------------

/// Open a fresh vsock connection, send `req`, collect all responses until a
/// terminal one, and return the full response list.
async fn call_agent(vm: &VzVm, req: &DaemonRequest) -> Result<Vec<DaemonResponse>> {
    let stream = connect_to_agent(vm, 60)
        .await
        .context("vsock: connect to agent")?;
    let mut proxy = VzProxy::new(stream);
    proxy.send_request(req).await.context("vsock: send request")
}

/// Return the last response in `responses`, or an error if the list is empty.
fn last_response(mut responses: Vec<DaemonResponse>) -> Result<DaemonResponse> {
    responses
        .pop()
        .context("vsock: agent returned no responses")
}

// ---------------------------------------------------------------------------
// VzRegistry
// ---------------------------------------------------------------------------

/// [`ImageRegistry`] adapter: forwards pull and layer queries to the VM daemon.
///
/// `has_image` and `get_image_layers` are fire-and-forget checks with no
/// matching protocol primitives — they conservatively return `false` / empty
/// so that the daemon's handler will pull as needed.  `pull_image` sends a
/// [`DaemonRequest::Pull`] and expects a [`DaemonResponse::Success`].
pub struct VzRegistry {
    vm: Arc<VzVm>,
}

impl VzRegistry {
    /// Create a new registry adapter backed by `vm`.
    pub fn new(vm: Arc<VzVm>) -> Self {
        Self { vm }
    }
}

impl AsAny for VzRegistry {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[async_trait]
impl ImageRegistry for VzRegistry {
    async fn has_image(&self, _name: &str, _tag: &str) -> bool {
        // No protocol primitive for a cache-check probe.  Return false so the
        // caller falls through to pull_image.
        false
    }

    async fn pull_image(&self, image_ref: &ImageRef) -> Result<ImageMetadata> {
        let req = DaemonRequest::Pull {
            image: format!("{}/{}", image_ref.namespace, image_ref.name),
            tag: Some(image_ref.tag.clone()),
        };

        let vm = Arc::clone(&self.vm);
        let responses = call_agent(&vm, &req).await?;

        let resp = last_response(responses)?;
        match resp {
            DaemonResponse::Success { .. } => Ok(ImageMetadata {
                name: format!("{}/{}", image_ref.namespace, image_ref.name),
                tag: image_ref.tag.clone(),
                layers: vec![],
            }),
            DaemonResponse::Error { message } => bail!("vz: pull_image failed: {message}"),
            other => bail!("vz: unexpected response to Pull: {other:?}"),
        }
    }

    fn get_image_layers(&self, _name: &str, _tag: &str) -> Result<Vec<PathBuf>> {
        // Layer paths live inside the VM's filesystem.  The host has no
        // meaningful way to enumerate them.  Returning an empty vec signals to
        // the daemon handler that it should pull first.
        Ok(vec![])
    }
}

// ---------------------------------------------------------------------------
// VzRuntime
// ---------------------------------------------------------------------------

/// [`ContainerRuntime`] adapter: forwards spawn to the VM daemon via `Run`.
///
/// `spawn_process` sends [`DaemonRequest::Run`] with `ephemeral: false` and
/// returns a synthetic [`SpawnResult`] with `pid = 0` — the actual PID lives
/// inside the VM and is not observable from the host.
pub struct VzRuntime {
    vm: Arc<VzVm>,
}

impl VzRuntime {
    /// Create a new runtime adapter backed by `vm`.
    pub fn new(vm: Arc<VzVm>) -> Self {
        Self { vm }
    }
}

impl AsAny for VzRuntime {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[async_trait]
impl ContainerRuntime for VzRuntime {
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            supports_user_namespaces: false,
            supports_cgroups_v2: true,
            supports_overlay_fs: true,
            supports_network_isolation: true,
            max_containers: None,
        }
    }

    async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<SpawnResult> {
        // Build command vec: [executable, args...]
        let mut command = vec![config.command.clone()];
        command.extend(config.args.clone());

        let req = DaemonRequest::Run {
            image: config
                .rootfs
                .to_str()
                .with_context(|| {
                    format!(
                        "rootfs path is not valid UTF-8: {}",
                        config.rootfs.display()
                    )
                })?
                .to_owned(),
            tag: None,
            command,
            memory_limit_bytes: None,
            cpu_weight: None,
            ephemeral: false,
            network: None,
            env: config.env.clone(),
            mounts: config.mounts.clone(),
            privileged: config.privileged,
            name: None,
        };

        let responses = call_agent(&self.vm, &req)
            .await
            .context("vz: spawn_process call_agent")?;

        let resp = last_response(responses)?;
        match resp {
            DaemonResponse::ContainerCreated { .. } => Ok(SpawnResult {
                pid: 0,
                output_reader: None,
            }),
            DaemonResponse::Error { message } => bail!("vz: spawn_process failed: {message}"),
            other => bail!("vz: unexpected response to Run: {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// VzFilesystem
// ---------------------------------------------------------------------------

/// [`FilesystemProvider`] adapter: filesystem operations are handled entirely
/// inside the VM.  All three methods are no-ops on the host side.
///
/// The VM's miniboxd sets up overlay mounts and performs `pivot_root` itself.
/// The host only needs to satisfy the trait contract so the hexagonal
/// architecture wiring compiles.
pub struct VzFilesystem {
    // Held for future use (e.g., querying VM state or virtiofs paths).
    #[allow(dead_code)]
    vm: Arc<VzVm>,
}

impl VzFilesystem {
    /// Create a new filesystem adapter backed by `vm`.
    pub fn new(vm: Arc<VzVm>) -> Self {
        Self { vm }
    }
}

impl AsAny for VzFilesystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl minibox_core::domain::RootfsSetup for VzFilesystem {
    fn setup_rootfs(
        &self,
        _image_layers: &[PathBuf],
        container_dir: &Path,
    ) -> Result<RootfsLayout> {
        // The VM daemon handles overlay setup internally.  Return the
        // container_dir as a placeholder path — it is not used by the host.
        tracing::debug!(
            container_dir = %container_dir.display(),
            "vz: setup_rootfs delegated to in-VM daemon (no-op on host)"
        );
        Ok(RootfsLayout {
            merged_dir: container_dir.to_path_buf(),
            rootfs_metadata: None,
            source_image_ref: None,
        })
    }

    fn cleanup(&self, container_dir: &Path) -> Result<()> {
        // Cleanup is handled by the VM daemon on container exit.
        tracing::debug!(
            container_dir = %container_dir.display(),
            "vz: filesystem cleanup delegated to in-VM daemon (no-op on host)"
        );
        Ok(())
    }
}

impl minibox_core::domain::ChildInit for VzFilesystem {
    fn pivot_root(&self, new_root: &Path) -> Result<()> {
        // pivot_root runs inside the VM's container process, not on the host.
        tracing::debug!(
            new_root = %new_root.display(),
            "vz: pivot_root delegated to in-VM daemon (no-op on host)"
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// VzLimiter
// ---------------------------------------------------------------------------

/// [`ResourceLimiter`] adapter: cgroup operations are handled inside the VM.
/// All three methods are no-ops on the host side.
///
/// The VM's miniboxd creates and manages cgroup directories for each container.
/// The host cannot see or manipulate the guest's cgroup hierarchy.
pub struct VzLimiter {
    // Held for future use (e.g., querying cgroup state inside the VM).
    #[allow(dead_code)]
    vm: Arc<VzVm>,
}

impl VzLimiter {
    /// Create a new limiter adapter backed by `vm`.
    pub fn new(vm: Arc<VzVm>) -> Self {
        Self { vm }
    }
}

impl AsAny for VzLimiter {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ResourceLimiter for VzLimiter {
    fn create(&self, container_id: &str, _config: &ResourceConfig) -> Result<String> {
        // Cgroup creation is handled inside the VM.
        tracing::debug!(
            container_id,
            "vz: resource limiter create delegated to in-VM daemon (no-op on host)"
        );
        Ok(container_id.to_owned())
    }

    fn add_process(&self, container_id: &str, pid: u32) -> Result<()> {
        // PID is inside the VM's PID namespace; not meaningful on the host.
        tracing::debug!(
            container_id,
            pid,
            "vz: add_process delegated to in-VM daemon (no-op on host)"
        );
        Ok(())
    }

    fn cleanup(&self, container_id: &str) -> Result<()> {
        // Cgroup cleanup is handled by the VM daemon.
        tracing::debug!(
            container_id,
            "vz: resource limiter cleanup delegated to in-VM daemon (no-op on host)"
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn _assert_image_registry<T: ImageRegistry>() {}
    fn _assert_container_runtime<T: ContainerRuntime>() {}
    fn _assert_filesystem_provider<T: FilesystemProvider>() {}
    fn _assert_resource_limiter<T: ResourceLimiter>() {}

    /// Compile-time check: all four adapters satisfy the required domain traits.
    ///
    /// This test body never executes — it only verifies the trait bounds compile.
    #[test]
    fn adapter_implements_all_traits() {
        // These function calls are zero-cost monomorphisations that fail to
        // compile if any trait impl is missing or has the wrong signature.
        let _ = _assert_image_registry::<VzRegistry>;
        let _ = _assert_container_runtime::<VzRuntime>;
        let _ = _assert_filesystem_provider::<VzFilesystem>;
        let _ = _assert_resource_limiter::<VzLimiter>;
    }
}
