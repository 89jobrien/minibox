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

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::domain::{ChildInit, RootfsSetup};
    use tempfile::TempDir;

    #[test]
    fn new_and_default_are_equivalent() {
        let _a = KrunFilesystem::new();
        let _b = KrunFilesystem::default();
    }

    #[test]
    fn setup_rootfs_returns_merged_dir_matching_container_dir() {
        let tmp = TempDir::new().expect("tempdir");
        let fs = KrunFilesystem::new();
        let layout = fs
            .setup_rootfs(&[], tmp.path())
            .expect("setup_rootfs should succeed");
        assert_eq!(layout.merged_dir, tmp.path().to_path_buf());
        assert!(layout.rootfs_metadata.is_none());
        assert!(layout.source_image_ref.is_none());
    }

    #[test]
    fn setup_rootfs_ignores_image_layers() {
        let tmp = TempDir::new().expect("tempdir");
        let fs = KrunFilesystem::new();
        let fake_layers = vec![
            PathBuf::from("/nonexistent/layer1"),
            PathBuf::from("/nonexistent/layer2"),
        ];
        let layout = fs
            .setup_rootfs(&fake_layers, tmp.path())
            .expect("setup_rootfs should succeed even with fake layers");
        assert_eq!(layout.merged_dir, tmp.path().to_path_buf());
    }

    #[test]
    fn setup_rootfs_errors_on_nonexistent_dir() {
        let fs = KrunFilesystem::new();
        let result = fs.setup_rootfs(&[], Path::new("/does/not/exist/xyzzy"));
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("container_dir"),
            "error should mention container_dir: {msg}"
        );
    }

    #[test]
    fn cleanup_is_noop_ok() {
        let tmp = TempDir::new().expect("tempdir");
        let fs = KrunFilesystem::new();
        fs.cleanup(tmp.path()).expect("cleanup should succeed");
    }

    #[test]
    fn cleanup_ok_even_for_nonexistent_path() {
        let fs = KrunFilesystem::new();
        fs.cleanup(Path::new("/does/not/exist"))
            .expect("cleanup should succeed for any path");
    }

    #[test]
    fn pivot_root_is_noop_ok() {
        let tmp = TempDir::new().expect("tempdir");
        let fs = KrunFilesystem::new();
        fs.pivot_root(tmp.path())
            .expect("pivot_root should succeed");
    }

    #[test]
    fn as_any_downcasts_to_self() {
        use minibox_core::domain::AsAny;
        let fs = KrunFilesystem::new();
        let any = fs.as_any();
        assert!(any.downcast_ref::<KrunFilesystem>().is_some());
    }
}
