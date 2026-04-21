//! Windows Host Compute Service (HCS) adapter stubs (Phase 2 — Windows).
//!
//! These stubs satisfy the [`ImageRegistry`], [`FilesystemProvider`],
//! [`ResourceLimiter`], and [`ContainerRuntime`] trait bounds so the crate
//! compiles on Windows today. Every method returns an error at runtime until
//! a real implementation is provided.
//!
//! Full implementation using the Windows HCS API (via the `hcs-rs` crate or
//! direct FFI to `hcsshim`) is planned for Phase 2.
//!
//! These adapters are **library-only** — they are not wired into `miniboxd`
//! and cannot be selected via `MINIBOX_ADAPTER`.

use anyhow::Result;
use async_trait::async_trait;
use minibox_core::{
    adapt,
    domain::{
        ContainerRuntime, ContainerSpawnConfig, ImageMetadata, ImageRegistry, ResourceConfig,
        ResourceLimiter, RootfsLayout, RuntimeCapabilities, SpawnResult,
    },
};
use std::path::{Path, PathBuf};

// ── HcsRuntime ───────────────────────────────────────────────────────────────

/// Windows Host Compute Service implementation of [`ContainerRuntime`] (Phase 2 stub).
///
/// Will delegate container spawning to HCS-managed Hyper-V utility VMs once
/// implemented.
pub struct HcsRuntime;

impl HcsRuntime {
    /// Create a new (stub) HCS runtime adapter.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ContainerRuntime for HcsRuntime {
    /// Return placeholder capabilities for the HCS runtime.
    ///
    /// User namespaces and cgroups v2 are Linux-specific and unavailable via
    /// HCS; overlay FS is similarly unavailable. Network isolation is listed as
    /// supported because HCS provides virtualised networking — this value will
    /// be refined when the full implementation lands.
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
        anyhow::bail!("HcsRuntime: not yet implemented (Phase 2)")
    }
}

// ── HcsFilesystem ────────────────────────────────────────────────────────────

/// Windows HCS implementation of [`FilesystemProvider`] (Phase 2 stub).
///
/// Will provide rootfs setup via WCOW layer stacking or VirtioFS when implemented.
pub struct HcsFilesystem;

impl HcsFilesystem {
    /// Create a new (stub) HCS filesystem adapter.
    pub fn new() -> Self {
        Self
    }
}

impl minibox_core::domain::RootfsSetup for HcsFilesystem {
    /// Not yet implemented — always returns an error.
    fn setup_rootfs(
        &self,
        _image_layers: &[PathBuf],
        _container_dir: &Path,
    ) -> Result<RootfsLayout> {
        anyhow::bail!("HcsFilesystem: not yet implemented (Phase 2)")
    }

    /// Not yet implemented — always returns an error.
    fn cleanup(&self, _container_dir: &Path) -> Result<()> {
        anyhow::bail!("HcsFilesystem: not yet implemented (Phase 2)")
    }
}

impl minibox_core::domain::ChildInit for HcsFilesystem {
    /// Not yet implemented — always returns an error.
    fn pivot_root(&self, _new_root: &Path) -> Result<()> {
        anyhow::bail!("HcsFilesystem: not yet implemented (Phase 2)")
    }
}

// ── HcsLimiter ───────────────────────────────────────────────────────────────

/// Windows HCS implementation of [`ResourceLimiter`] (Phase 2 stub).
///
/// Will delegate resource limits to Windows Job Objects or HCS compute system
/// settings when implemented.
pub struct HcsLimiter;

impl HcsLimiter {
    /// Create a new (stub) HCS resource limiter adapter.
    pub fn new() -> Self {
        Self
    }
}

impl ResourceLimiter for HcsLimiter {
    /// Not yet implemented — always returns an error.
    fn create(&self, _container_id: &str, _config: &ResourceConfig) -> Result<String> {
        anyhow::bail!("HcsLimiter: not yet implemented (Phase 2)")
    }

    /// Not yet implemented — always returns an error.
    fn add_process(&self, _container_id: &str, _pid: u32) -> Result<()> {
        anyhow::bail!("HcsLimiter: not yet implemented (Phase 2)")
    }

    /// Not yet implemented — always returns an error.
    fn cleanup(&self, _container_id: &str) -> Result<()> {
        anyhow::bail!("HcsLimiter: not yet implemented (Phase 2)")
    }
}

// ── HcsRegistry ──────────────────────────────────────────────────────────────

/// Windows HCS implementation of [`ImageRegistry`] (Phase 2 stub).
///
/// Will pull images into a local Windows container image store when implemented.
pub struct HcsRegistry;

impl HcsRegistry {
    /// Create a new (stub) HCS image registry adapter.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ImageRegistry for HcsRegistry {
    /// Always returns `false` — no images are available in this stub.
    async fn has_image(&self, _name: &str, _tag: &str) -> bool {
        false
    }

    /// Not yet implemented — always returns an error.
    async fn pull_image(
        &self,
        _image_ref: &crate::image::reference::ImageRef,
    ) -> Result<ImageMetadata> {
        anyhow::bail!("HcsRegistry: not yet implemented (Phase 2)")
    }

    /// Not yet implemented — always returns an error.
    fn get_image_layers(&self, _name: &str, _tag: &str) -> Result<Vec<PathBuf>> {
        anyhow::bail!("HcsRegistry: not yet implemented (Phase 2)")
    }
}

// Register all four HCS adapters with the adapt! macro so they satisfy the
// AsAny + Default bounds required by the daemon's adapter registry.
adapt!(HcsRuntime, HcsFilesystem, HcsLimiter, HcsRegistry);
