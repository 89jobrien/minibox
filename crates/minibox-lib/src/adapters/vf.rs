//! Virtualization Framework adapter stubs (Phase 2 — macOS).
//!
//! These stubs satisfy the trait bounds so the crate compiles on macOS.
//! Full implementation using Apple's Virtualization.framework is planned
//! for Phase 2.

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

/// macOS Virtualization Framework runtime (Phase 2 stub).
pub struct VfRuntime;

impl VfRuntime {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ContainerRuntime for VfRuntime {
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
        anyhow::bail!("VfRuntime: not yet implemented (Phase 2)")
    }
}

// ── VfFilesystem ─────────────────────────────────────────────────────────────

/// macOS Virtualization Framework filesystem provider (Phase 2 stub).
pub struct VfFilesystem;

impl VfFilesystem {
    pub fn new() -> Self {
        Self
    }
}

impl FilesystemProvider for VfFilesystem {
    fn setup_rootfs(&self, _image_layers: &[PathBuf], _container_dir: &Path) -> Result<PathBuf> {
        anyhow::bail!("VfFilesystem: not yet implemented (Phase 2)")
    }

    fn pivot_root(&self, _new_root: &Path) -> Result<()> {
        anyhow::bail!("VfFilesystem: not yet implemented (Phase 2)")
    }

    fn cleanup(&self, _container_dir: &Path) -> Result<()> {
        anyhow::bail!("VfFilesystem: not yet implemented (Phase 2)")
    }
}

// ── VfLimiter ────────────────────────────────────────────────────────────────

/// macOS Virtualization Framework resource limiter (Phase 2 stub).
pub struct VfLimiter;

impl VfLimiter {
    pub fn new() -> Self {
        Self
    }
}

impl ResourceLimiter for VfLimiter {
    fn create(&self, _container_id: &str, _config: &ResourceConfig) -> Result<String> {
        anyhow::bail!("VfLimiter: not yet implemented (Phase 2)")
    }

    fn add_process(&self, _container_id: &str, _pid: u32) -> Result<()> {
        anyhow::bail!("VfLimiter: not yet implemented (Phase 2)")
    }

    fn cleanup(&self, _container_id: &str) -> Result<()> {
        anyhow::bail!("VfLimiter: not yet implemented (Phase 2)")
    }
}

// ── VfRegistry ───────────────────────────────────────────────────────────────

/// macOS Virtualization Framework image registry (Phase 2 stub).
pub struct VfRegistry;

impl VfRegistry {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ImageRegistry for VfRegistry {
    async fn has_image(&self, _name: &str, _tag: &str) -> bool {
        false
    }

    async fn pull_image(&self, _name: &str, _tag: &str) -> Result<ImageMetadata> {
        anyhow::bail!("VfRegistry: not yet implemented (Phase 2)")
    }

    fn get_image_layers(&self, _name: &str, _tag: &str) -> Result<Vec<PathBuf>> {
        anyhow::bail!("VfRegistry: not yet implemented (Phase 2)")
    }
}

adapt!(VfRuntime, VfFilesystem, VfLimiter, VfRegistry);
