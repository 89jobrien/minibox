//! Container filesystem ports, bind mounts, and rootfs metadata.

use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::AsAny;

/// A host-path bind mount to inject into a container at startup.
///
/// `host_path` is canonicalized and validated before the mount is applied.
/// `container_path` must be absolute (starts with `/`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BindMount {
    /// Absolute path on the host to mount into the container.
    pub host_path: std::path::PathBuf,
    /// Absolute path inside the container where the host path is mounted.
    pub container_path: std::path::PathBuf,
    /// If `true`, the mount is read-only inside the container.
    pub read_only: bool,
}

impl BindMount {
    /// Parse a `-v src:dst[:ro]` volume shorthand into a `BindMount`.
    ///
    /// # Errors
    ///
    /// Returns an error if the format is invalid, paths are not absolute, or
    /// paths contain `..` components.
    pub fn parse_volume(s: &str) -> anyhow::Result<Self> {
        let parts: Vec<&str> = s.splitn(3, ':').collect();
        if parts.len() < 2 {
            anyhow::bail!("invalid volume format {s:?}: expected src:dst or src:dst:ro");
        }
        let host_path = std::path::PathBuf::from(parts[0]);
        if !host_path.is_absolute() {
            anyhow::bail!(
                "host path {} must be absolute (start with /)",
                host_path.display()
            );
        }
        if host_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            anyhow::bail!(
                "host path {} must not contain '..' components",
                host_path.display()
            );
        }
        let container_path = std::path::PathBuf::from(parts[1]);
        if !container_path.is_absolute() {
            anyhow::bail!(
                "container path {} must be absolute (start with /)",
                container_path.display()
            );
        }
        let read_only = parts.get(2).is_some_and(|f| *f == "ro");
        Ok(Self {
            host_path,
            container_path,
            read_only,
        })
    }

    /// Parse a `--mount type=bind,src=PATH,dst=PATH[,readonly]` spec into a `BindMount`.
    ///
    /// # Errors
    ///
    /// Returns an error if the mount type is unsupported, required keys are
    /// missing, or paths are not absolute.
    pub fn parse_mount(s: &str) -> anyhow::Result<Self> {
        let mut mount_type = None::<String>;
        let mut src = None::<std::path::PathBuf>;
        let mut dst = None::<std::path::PathBuf>;
        let mut read_only = false;

        for kv in s.split(',') {
            if kv == "readonly" || kv == "ro" {
                read_only = true;
                continue;
            }
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            match k {
                "type" => mount_type = Some(v.to_string()),
                "src" | "source" => src = Some(std::path::PathBuf::from(v)),
                "dst" | "target" | "destination" => dst = Some(std::path::PathBuf::from(v)),
                _ => {}
            }
        }

        match mount_type.as_deref() {
            Some("bind") | None => {}
            Some(t) => anyhow::bail!("unsupported mount type {t:?}: only 'bind' is supported"),
        }

        let host_path = src.ok_or_else(|| anyhow::anyhow!("--mount missing 'src' key"))?;
        if !host_path.is_absolute() {
            anyhow::bail!(
                "host path {} must be absolute (start with /)",
                host_path.display()
            );
        }
        if host_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            anyhow::bail!(
                "host path {} must not contain '..' components",
                host_path.display()
            );
        }
        let container_path = dst.ok_or_else(|| anyhow::anyhow!("--mount missing 'dst' key"))?;
        if !container_path.is_absolute() {
            anyhow::bail!(
                "container path {} must be absolute (start with /)",
                container_path.display()
            );
        }

        Ok(Self {
            host_path,
            container_path,
            read_only,
        })
    }
}

/// How the Linux adapter should make `/var/run` share the fresh `/run` tmpfs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarRunMountStrategy {
    /// Keep an image-provided `/var/run -> ../run` symlink.
    ExistingSymlink,
    /// Bind the fresh `/run` mount over an image-provided directory.
    BindToRun,
    /// Create the conventional `/var/run -> ../run` symlink.
    CreateSymlink,
}

/// Select the safe `/var/run` setup without inspecting host paths in the domain layer.
#[must_use]
pub const fn var_run_mount_strategy(
    exists: bool,
    is_symlink: bool,
    points_to_run: bool,
) -> VarRunMountStrategy {
    if is_symlink && points_to_run {
        VarRunMountStrategy::ExistingSymlink
    } else if exists {
        VarRunMountStrategy::BindToRun
    } else {
        VarRunMountStrategy::CreateSymlink
    }
}

/// Return whether a Linux release supports `mount_setattr(MOUNT_ATTR_IDMAP)`.
#[must_use]
pub fn kernel_supports_idmapped_mounts(release: &str) -> bool {
    let mut parts = release.split(|c: char| c == char::from(46) || c == char::from(45));
    let major = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    let minor = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    (major, minor) >= (5, 12)
}

// ---------------------------------------------------------------------------
// Filesystem Provider Port
// ---------------------------------------------------------------------------

/// Abstraction for container filesystem operations.
///
/// This trait defines the contract for filesystem implementations.
/// Daemon-side filesystem lifecycle: setup container rootfs and cleanup after
/// exit.
///
/// Implementations might include overlay filesystem, bind mounts, or
/// other copy-on-write filesystems like ZFS or Btrfs.
///
/// # Security
///
/// Implementations MUST:
/// - Validate all paths to prevent traversal attacks
/// - Mount filesystems with appropriate security flags (nosuid, nodev)
/// - Properly clean up mounts to avoid resource leaks
pub trait RootfsSetup: AsAny + Send + Sync {
    /// Setup the container rootfs and return the merged directory plus any
    /// backend metadata needed by follow-on operations such as commit/build.
    ///
    /// Creates the necessary directory structure and mounts (e.g., overlay)
    /// to provide a writable rootfs for the container.
    ///
    /// # Arguments
    ///
    /// * `image_layers` - Ordered list of layer paths (bottom-to-top)
    /// * `container_dir` - Per-container working directory
    ///
    /// # Returns
    ///
    /// A [`RootfsLayout`] describing the merged rootfs and optional
    /// backend-specific metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Path validation fails (security)
    /// - Directory creation fails
    /// - Mount operation fails
    ///
    /// # Security
    ///
    /// MUST validate that `image_layers` paths don't contain `..` or
    /// escape the allowed base directory.
    fn setup_rootfs(&self, image_layers: &[PathBuf], container_dir: &Path) -> Result<RootfsLayout>;

    /// Cleanup mounts after container exit.
    ///
    /// Unmounts the rootfs and removes the per-container directories.
    ///
    /// # Arguments
    ///
    /// * `container_dir` - Per-container directory to clean up
    ///
    /// # Errors
    ///
    /// Returns an error if unmount or directory removal fails.
    fn cleanup(&self, container_dir: &Path) -> Result<()>;
}

/// Child-process filesystem initialisation: pivot root inside the cloned
/// container process.
///
/// This trait is received only by the container child process after
/// `clone(2)`, keeping daemon-side setup (`RootfsSetup`) and child-side
/// init (`ChildInit`) under separate ownership.
pub trait ChildInit: Send + Sync {
    /// Pivot root inside the container process.
    ///
    /// This is called **inside the cloned child process** to switch the
    /// root filesystem and set up essential pseudo-filesystems (proc, sys, dev).
    ///
    /// # Arguments
    ///
    /// * `new_root` - Path to the new root filesystem
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Bind mount fails
    /// - Essential filesystem mounts fail
    /// - `pivot_root` syscall fails
    /// - Old root unmount fails
    ///
    /// # Security
    ///
    /// MUST mount proc/sys/dev with appropriate security flags:
    /// - proc: nosuid, nodev, noexec
    /// - sys: rdonly, nosuid, nodev, noexec
    /// - dev: nosuid, noexec
    fn pivot_root(&self, new_root: &Path) -> Result<()>;
}

/// Combined filesystem provider: supertrait alias that bundles [`RootfsSetup`]
/// and [`ChildInit`] for adapters that implement both lifecycle phases.
///
/// Prefer using [`RootfsSetup`] or [`ChildInit`] directly at call sites that
/// only need one half of the lifecycle.
pub trait FilesystemProvider: RootfsSetup + ChildInit + Send + Sync {}

/// Blanket implementation: any type that implements both [`RootfsSetup`] and
/// [`ChildInit`] automatically satisfies [`FilesystemProvider`].
impl<T: RootfsSetup + ChildInit> FilesystemProvider for T {}

/// Backend-specific writable-layer metadata produced by [`RootfsSetup::setup_rootfs`].
///
/// Persisted into [`ContainerRecord`] so that commit/build logic can locate
/// the writable layer without re-querying the container runtime.
///
/// The `metadata` map carries backend-specific key/value pairs so that new
/// backends can encode their own data (e.g. `"colima_instance" => "colima"`)
/// without adding new enum variants (OCP).  Callers that only need the
/// host-visible upper directory should use
/// [`BackendRootfsMetadata::overlay_upper_dir`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackendRootfsMetadata {
    /// Overlay filesystem backend.  `upper_dir` is the host-visible (or
    /// guest-visible, for VM adapters) writable layer directory.
    /// `metadata` carries adapter-specific key/value pairs, e.g.:
    /// - `"colima_instance"` — Lima/Colima instance name
    Overlay {
        /// Writable overlay layer path visible to the backend.
        upper_dir: crate::path::InternalPath,
        /// Adapter-specific rootfs metadata.
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        metadata: HashMap<String, String>,
    },
}

impl BackendRootfsMetadata {
    /// Return the host-visible overlay upper directory.
    #[must_use]
    pub const fn overlay_upper_dir(&self) -> &crate::path::InternalPath {
        match self {
            Self::Overlay { upper_dir, .. } => upper_dir,
        }
    }

    /// Return an adapter-specific metadata value by key.
    #[must_use]
    pub fn metadata_value(&self, key: &str) -> Option<&str> {
        match self {
            Self::Overlay { metadata, .. } => metadata.get(key).map(String::as_str),
        }
    }
}

/// Filesystem layout returned by [`FilesystemProvider::setup_rootfs`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootfsLayout {
    /// Path to the merged/mounted rootfs that the runtime will use.
    pub merged_dir: crate::path::InternalPath,
    /// Typed backend metadata for the writable layer, when the backend exposes
    /// one.  `None` for copy-based (GKE/proot) and VZ (in-VM) backends.
    pub rootfs_metadata: Option<BackendRootfsMetadata>,
    /// Source image reference associated with this rootfs when known.
    pub source_image_ref: Option<String>,
}

#[cfg(test)]
mod idmapped_mount_tests {
    use super::{VarRunMountStrategy, var_run_mount_strategy};

    #[test]
    fn runtime_directory_plan_hides_existing_var_run_directories() {
        assert_eq!(
            var_run_mount_strategy(true, false, false),
            VarRunMountStrategy::BindToRun
        );
        assert_eq!(
            var_run_mount_strategy(true, true, true),
            VarRunMountStrategy::ExistingSymlink
        );
        assert_eq!(
            var_run_mount_strategy(false, false, false),
            VarRunMountStrategy::CreateSymlink
        );
    }

    use super::kernel_supports_idmapped_mounts;

    #[test]
    fn rejects_kernels_before_5_12() {
        assert!(!kernel_supports_idmapped_mounts("5.11.19"));
    }

    #[test]
    fn accepts_5_12_and_newer_kernels() {
        assert!(kernel_supports_idmapped_mounts("5.12.0"));
        assert!(kernel_supports_idmapped_mounts("6.10.3-custom"));
    }
}
