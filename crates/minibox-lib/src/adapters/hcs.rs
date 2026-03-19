//! Windows Host Compute Service (HCS) adapter stubs (Phase 2 — Windows).
//!
//! These stubs satisfy the trait bounds so the crate compiles on Windows.
//! Full implementation using the Windows HCS API is planned for Phase 2.

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

// ── HcsRuntime ───────────────────────────────────────────────────────────────

/// Windows HCS container runtime (Phase 2 stub).
pub struct HcsRuntime;

impl HcsRuntime {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ContainerRuntime for HcsRuntime {
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            supports_user_namespaces: false,
            supports_cgroups_v2: false,
            supports_overlay_fs: false,
            supports_network_isolation: true,
            max_containers: None,
        }
    }

    async fn spawn_process(&self, _config: &ContainerSpawnConfig) -> Result<SpawnResult> {
        anyhow::bail!("HcsRuntime: not yet implemented (Phase 2)")
    }
}

// ── HcsFilesystem ────────────────────────────────────────────────────────────

/// Windows HCS filesystem provider (Phase 2 stub).
pub struct HcsFilesystem;

impl HcsFilesystem {
    pub fn new() -> Self {
        Self
    }
}

impl FilesystemProvider for HcsFilesystem {
    fn setup_rootfs(&self, _image_layers: &[PathBuf], _container_dir: &Path) -> Result<PathBuf> {
        anyhow::bail!("HcsFilesystem: not yet implemented (Phase 2)")
    }

    fn pivot_root(&self, _new_root: &Path) -> Result<()> {
        anyhow::bail!("HcsFilesystem: not yet implemented (Phase 2)")
    }

    fn cleanup(&self, _container_dir: &Path) -> Result<()> {
        anyhow::bail!("HcsFilesystem: not yet implemented (Phase 2)")
    }
}

// ── HcsLimiter ───────────────────────────────────────────────────────────────

/// Windows HCS resource limiter (Phase 2 stub).
pub struct HcsLimiter;

impl HcsLimiter {
    pub fn new() -> Self {
        Self
    }
}

impl ResourceLimiter for HcsLimiter {
    fn create(&self, _container_id: &str, _config: &ResourceConfig) -> Result<String> {
        anyhow::bail!("HcsLimiter: not yet implemented (Phase 2)")
    }

    fn add_process(&self, _container_id: &str, _pid: u32) -> Result<()> {
        anyhow::bail!("HcsLimiter: not yet implemented (Phase 2)")
    }

    fn cleanup(&self, _container_id: &str) -> Result<()> {
        anyhow::bail!("HcsLimiter: not yet implemented (Phase 2)")
    }
}

// ── HcsRegistry ──────────────────────────────────────────────────────────────

/// Windows HCS image registry (Phase 2 stub).
pub struct HcsRegistry;

impl HcsRegistry {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ImageRegistry for HcsRegistry {
    async fn has_image(&self, _name: &str, _tag: &str) -> bool {
        false
    }

    async fn pull_image(&self, _name: &str, _tag: &str) -> Result<ImageMetadata> {
        anyhow::bail!("HcsRegistry: not yet implemented (Phase 2)")
    }

    fn get_image_layers(&self, _name: &str, _tag: &str) -> Result<Vec<PathBuf>> {
        anyhow::bail!("HcsRegistry: not yet implemented (Phase 2)")
    }
}

adapt!(HcsRuntime, HcsFilesystem, HcsLimiter, HcsRegistry);
