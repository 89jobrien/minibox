//! Overlay filesystem adapter implementing the FilesystemProvider trait.
//!
//! This adapter wraps the existing overlay filesystem implementation from
//! [`crate::container::filesystem`] to implement the domain's
//! [`FilesystemProvider`] trait.

use crate::container::filesystem;
use crate::domain::FilesystemProvider;
use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Overlay filesystem implementation of the [`FilesystemProvider`] trait.
///
/// This adapter uses Linux overlay filesystem (overlayfs) to provide
/// copy-on-write layer stacking for container rootfs. It delegates to the
/// existing filesystem module which handles the low-level mount operations.
///
/// # Platform Support
///
/// This adapter is **Linux-only** and requires:
/// - Kernel 4.0+ (5.0+ recommended)
/// - `CONFIG_OVERLAY_FS=y` kernel configuration
/// - Root privileges for mount operations
///
/// # Security
///
/// - All paths are validated to prevent traversal attacks
/// - Filesystems are mounted with security flags (nosuid, nodev)
/// - Essential pseudo-filesystems (proc, sys, dev) are mounted read-only
///   where appropriate
///
/// # Example
///
/// ```rust,ignore
/// use minibox_lib::adapters::OverlayFilesystem;
/// use minibox_lib::domain::FilesystemProvider;
/// use std::path::PathBuf;
///
/// let fs = OverlayFilesystem;
/// let layers = vec![
///     PathBuf::from("/var/lib/minibox/images/alpine/layer1"),
///     PathBuf::from("/var/lib/minibox/images/alpine/layer2"),
/// ];
/// let container_dir = PathBuf::from("/var/lib/minibox/containers/abc123");
///
/// // Setup overlay (creates merged/ with writable layer)
/// let rootfs = fs.setup_rootfs(&layers, &container_dir)?;
///
/// // Later, cleanup
/// fs.cleanup(&container_dir)?;
/// ```
#[derive(Debug, Clone, Copy)]
pub struct OverlayFilesystem;

impl OverlayFilesystem {
    /// Create a new overlay filesystem adapter.
    ///
    /// This is a zero-sized type, so construction is trivial.
    pub fn new() -> Self {
        Self
    }
}

impl Default for OverlayFilesystem {
    fn default() -> Self {
        Self::new()
    }
}

impl FilesystemProvider for OverlayFilesystem {
    fn setup_rootfs(
        &self,
        image_layers: &[PathBuf],
        container_dir: &Path,
    ) -> Result<PathBuf> {
        debug!(
            "setting up overlay rootfs with {} layers at {:?}",
            image_layers.len(),
            container_dir
        );

        // Delegate to existing filesystem implementation
        filesystem::setup_overlay(image_layers, container_dir)
    }

    fn pivot_root(&self, new_root: &Path) -> Result<()> {
        debug!("pivoting root to {:?}", new_root);

        // Delegate to existing filesystem implementation
        // IMPORTANT: This must be called from inside the container process
        // after namespaces have been created
        filesystem::pivot_root_to(new_root)
    }

    fn cleanup(&self, container_dir: &Path) -> Result<()> {
        debug!("cleaning up filesystem mounts at {:?}", container_dir);

        // Delegate to existing filesystem implementation
        filesystem::cleanup_mounts(container_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filesystem_creation() {
        let fs = OverlayFilesystem::new();
        // Zero-sized type, just verify it compiles
        let _ = fs;
    }

    #[test]
    fn test_filesystem_default() {
        let fs = OverlayFilesystem::default();
        let _ = fs;
    }

    // Note: Actual setup_rootfs tests require Linux with root privileges
    // and are better suited for integration tests
}
