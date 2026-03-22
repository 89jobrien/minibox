//! Apple Virtualization.framework adapter stubs (Phase 2 — macOS).
//!
//! These stubs satisfy the [`ImageRegistry`], [`FilesystemProvider`],
//! [`ResourceLimiter`], and [`ContainerRuntime`] trait bounds so the crate
//! compiles on macOS today. Every method returns an error at runtime until
//! a real implementation is provided.
//!
//! Full implementation using Apple's `Virtualization.framework` API (via
//! `apple-vf` or a native FFI layer) is planned for Phase 2. The framework
//! is available on macOS 11.0+.
//!
//! These adapters are **library-only** — they are not wired into `miniboxd`
//! and cannot be selected via `MINIBOX_ADAPTER`.

use crate::{
    adapt,
    domain::{
        ContainerRuntime, ContainerSpawnConfig, FilesystemProvider, ImageMetadata, ImageRegistry,
        ResourceConfig, ResourceLimiter, RuntimeCapabilities, SpawnResult,
    },
};
use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};

// ── VfRuntime ────────────────────────────────────────────────────────────────

/// Apple Virtualization.framework implementation of [`ContainerRuntime`] (Phase 2 stub).
///
/// Will delegate container spawning to a lightweight Linux VM managed via the
/// `Virtualization.framework` APIs once implemented.
pub struct VfRuntime;

impl VfRuntime {
    /// Create a new (stub) VF runtime adapter.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ContainerRuntime for VfRuntime {
    /// Return placeholder capabilities for the VF runtime.
    ///
    /// User namespaces and cgroups v2 are unavailable on macOS; overlay FS is
    /// also not natively supported. Network isolation is listed as supported
    /// because Virtualization.framework provides virtualised networking — this
    /// value will be refined when the full implementation lands.
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            supports_user_namespaces: false,
            supports_cgroups_v2: false,
            supports_overlay_fs: false,
            supports_network_isolation: true,
            max_containers: None,
        }
    }

    /// Not yet implemented — always returns an error.
    async fn spawn_process(&self, _config: &ContainerSpawnConfig) -> Result<SpawnResult> {
        anyhow::bail!("VfRuntime: not yet implemented (Phase 2)")
    }
}

// ── VfFilesystem ─────────────────────────────────────────────────────────────

/// Apple Virtualization.framework implementation of [`FilesystemProvider`] (Phase 2 stub).
///
/// Will provide rootfs setup via VirtioFS or similar when implemented.
pub struct VfFilesystem;

impl VfFilesystem {
    /// Create a new (stub) VF filesystem adapter.
    pub fn new() -> Self {
        Self
    }
}

impl FilesystemProvider for VfFilesystem {
    /// Not yet implemented — always returns an error.
    fn setup_rootfs(&self, _image_layers: &[PathBuf], _container_dir: &Path) -> Result<PathBuf> {
        anyhow::bail!("VfFilesystem: not yet implemented (Phase 2)")
    }

    /// Not yet implemented — always returns an error.
    fn pivot_root(&self, _new_root: &Path) -> Result<()> {
        anyhow::bail!("VfFilesystem: not yet implemented (Phase 2)")
    }

    /// Not yet implemented — always returns an error.
    fn cleanup(&self, _container_dir: &Path) -> Result<()> {
        anyhow::bail!("VfFilesystem: not yet implemented (Phase 2)")
    }
}

// ── VfLimiter ────────────────────────────────────────────────────────────────

/// Apple Virtualization.framework implementation of [`ResourceLimiter`] (Phase 2 stub).
///
/// Will delegate resource limits to the VM's cgroup hierarchy when implemented.
pub struct VfLimiter;

impl VfLimiter {
    /// Create a new (stub) VF resource limiter adapter.
    pub fn new() -> Self {
        Self
    }
}

impl ResourceLimiter for VfLimiter {
    /// Not yet implemented — always returns an error.
    fn create(&self, _container_id: &str, _config: &ResourceConfig) -> Result<String> {
        anyhow::bail!("VfLimiter: not yet implemented (Phase 2)")
    }

    /// Not yet implemented — always returns an error.
    fn add_process(&self, _container_id: &str, _pid: u32) -> Result<()> {
        anyhow::bail!("VfLimiter: not yet implemented (Phase 2)")
    }

    /// Not yet implemented — always returns an error.
    fn cleanup(&self, _container_id: &str) -> Result<()> {
        anyhow::bail!("VfLimiter: not yet implemented (Phase 2)")
    }
}

// ── VfRegistry ───────────────────────────────────────────────────────────────

/// Apple Virtualization.framework implementation of [`ImageRegistry`] (Phase 2 stub).
///
/// Will pull images into the VM's local container store when implemented.
pub struct VfRegistry;

impl VfRegistry {
    /// Create a new (stub) VF image registry adapter.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ImageRegistry for VfRegistry {
    /// Always returns `false` — no images are available in this stub.
    async fn has_image(&self, _name: &str, _tag: &str) -> bool {
        false
    }

    /// Not yet implemented — always returns an error.
    async fn pull_image(&self, _name: &str, _tag: &str) -> Result<ImageMetadata> {
        anyhow::bail!("VfRegistry: not yet implemented (Phase 2)")
    }

    /// Not yet implemented — always returns an error.
    fn get_image_layers(&self, _name: &str, _tag: &str) -> Result<Vec<PathBuf>> {
        anyhow::bail!("VfRegistry: not yet implemented (Phase 2)")
    }
}

// Register all four VF adapters with the adapt! macro so they satisfy the
// AsAny + Default bounds required by the daemon's adapter registry.
adapt!(VfRuntime, VfFilesystem, VfLimiter, VfRegistry);
