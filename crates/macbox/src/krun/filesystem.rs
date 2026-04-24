//! `KrunFilesystem` — filesystem adapter for the krun/smolvm VM backend.
//!
//! Since the VM manages its own root filesystem init (`pivot_root` is handled
//! inside the guest), both `setup_rootfs` and `pivot_root` are minimal
//! host-side stubs.  `setup_rootfs` validates that the container directory
//! exists and is accessible; `pivot_root` is a no-op returning `Ok(())`.

use anyhow::{Context, Result};
use minibox_core::domain::{ChildInit, RootfsLayout, RootfsSetup};
use std::path::{Path, PathBuf};

/// Filesystem adapter for the krun microVM backend.
///
/// The VM manages its own root filesystem — no overlay mounts or `pivot_root`
/// are performed on the host side.
pub struct KrunFilesystem;

impl KrunFilesystem {
    /// Create a new `KrunFilesystem` adapter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for KrunFilesystem {
    fn default() -> Self {
        Self::new()
    }
}

impl minibox_core::domain::AsAny for KrunFilesystem {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl RootfsSetup for KrunFilesystem {
    /// Validate that `container_dir` exists and return a minimal layout.
    ///
    /// No mounts are performed — the VM's image provides the rootfs.
    fn setup_rootfs(
        &self,
        _image_layers: &[PathBuf],
        container_dir: &Path,
    ) -> Result<RootfsLayout> {
        // Validate that the container directory exists and is accessible.
        std::fs::metadata(container_dir).with_context(|| {
            format!(
                "krun: container_dir does not exist or is not accessible: {}",
                container_dir.display()
            )
        })?;

        tracing::debug!(
            rootfs = %container_dir.display(),
            "krun: filesystem setup_rootfs validated container_dir"
        );

        Ok(RootfsLayout {
            merged_dir: container_dir.to_path_buf(),
            rootfs_metadata: None,
            source_image_ref: None,
        })
    }

    /// No-op cleanup — no host mounts to tear down.
    fn cleanup(&self, _container_dir: &Path) -> Result<()> {
        Ok(())
    }
}

impl ChildInit for KrunFilesystem {
    /// No-op — the VM kernel handles root filesystem initialisation internally.
    fn pivot_root(&self, _new_root: &Path) -> Result<()> {
        Ok(())
    }
}
