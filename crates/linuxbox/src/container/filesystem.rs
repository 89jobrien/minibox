//! Overlay filesystem setup and `pivot_root` for container isolation.
//!
//! The main entry points are:
//! - [`setup_overlay`] -- mounts an overlay fs from image layers and returns the
//!   merged directory path.
//! - [`pivot_root_to`] -- called inside the child process to switch the root
//!   filesystem and mount essential pseudo-filesystems.
//! - [`cleanup_mounts`] -- called after container exit to unmount and clean up.

use crate::error::FilesystemError;
use anyhow::Context;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

#[cfg(target_os = "linux")]
use nix::mount::{MntFlags, MsFlags, mount, umount2};

#[cfg(not(target_os = "linux"))]
compile_error!("minibox only supports Linux");

/// Return `true` if `path` contains any `..` (parent directory) component.
///
/// Used as a fast pre-check before canonicalization so that clearly malicious
/// paths are rejected without touching the filesystem.
fn has_parent_dir_component(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

// ---------------------------------------------------------------------------
// Path validation (security)
// ---------------------------------------------------------------------------

/// Validate that a layer path is safe and falls within `base_dir`.
///
/// # Security
///
/// Prevents path traversal attacks by applying two independent checks:
///
/// 1. **Component scan** — rejects paths that contain any `..` (ParentDir)
///    component before any filesystem access occurs.
/// 2. **Canonicalization** — resolves symlinks and verifies that the resulting
///    absolute path has `base_dir` (also canonicalized) as a prefix, catching
///    traversal via intermediate symlinks.
///
/// Both checks must pass for the path to be considered safe. The function
/// returns an error whose message includes `"path traversal"` so callers and
/// tests can match on it reliably.
pub(crate) fn validate_layer_path(path: &Path, base_dir: &Path) -> anyhow::Result<()> {
    // Reject paths with parent directory components
    if has_parent_dir_component(path) {
        anyhow::bail!(
            "path traversal attempt: layer path contains '..' component: {:?}",
            path
        );
    }

    // Canonicalize both paths to resolve symlinks
    let canonical_path = path
        .canonicalize()
        .with_context(|| format!("canonicalizing layer path {:?}", path))?;

    let canonical_base = base_dir
        .canonicalize()
        .or_else(|_| {
            // Base dir might not exist yet, that's ok
            fs::create_dir_all(base_dir)?;
            base_dir.canonicalize()
        })
        .with_context(|| format!("canonicalizing base dir {:?}", base_dir))?;

    // Verify the layer path is within the base directory
    if !canonical_path.starts_with(&canonical_base) {
        anyhow::bail!(
            "path traversal attempt: layer {:?} is outside allowed directory {:?}",
            canonical_path,
            canonical_base
        );
    }

    debug!(canonical_path = %canonical_path.display(), "filesystem: layer path validated");
    Ok(())
}

// ---------------------------------------------------------------------------
// Overlay setup (parent process)
// ---------------------------------------------------------------------------

/// Set up an overlay filesystem for a container.
///
/// Given ordered image layers (bottom-to-top) and a per-container working
/// directory, this function:
/// 1. Creates `{container_dir}/merged/`, `upper/`, and `work/` directories.
/// 2. Constructs the `lowerdir=` string from the layers in the correct order
///    (overlayfs wants them listed top-to-bottom, i.e. reversed).
/// 3. Mounts the overlay and returns the path to `merged/`.
///
/// # Security
///
/// All layer paths are validated to prevent path traversal attacks. Paths
/// containing `..`, symlinks, or absolute references are rejected.
pub fn setup_overlay(image_layers: &[PathBuf], container_dir: &Path) -> anyhow::Result<PathBuf> {
    // Default base for production.
    let images_base = PathBuf::from("/var/lib/minibox/images");
    setup_overlay_with_base(image_layers, container_dir, &images_base)
}

/// Set up an overlay filesystem for a container using a custom images base.
///
/// This is used by tests and other callers that store images outside the
/// default `/var/lib/minibox/images` directory.
pub fn setup_overlay_with_base(
    image_layers: &[PathBuf],
    container_dir: &Path,
    images_base: &Path,
) -> anyhow::Result<PathBuf> {
    let merged = container_dir.join("merged");
    let upper = container_dir.join("upper");
    let work = container_dir.join("work");

    for dir in [&merged, &upper, &work] {
        fs::create_dir_all(dir).map_err(|source| FilesystemError::CreateDir {
            path: dir.display().to_string(),
            source,
        })?;
    }

    // SECURITY: Validate all layer paths to prevent path traversal.
    for layer_path in image_layers {
        validate_layer_path(layer_path, images_base)
            .with_context(|| format!("validating layer path {:?}", layer_path))?;
    }

    // overlayfs lowerdir lists layers from **top** (most recent) to **bottom**
    // (oldest). The caller provides them bottom-to-top, so we reverse here.
    let lowerdir: String = image_layers
        .iter()
        .rev()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(":");

    let options = format!(
        "lowerdir={lowerdir},upperdir={upper},workdir={work}",
        upper = upper.display(),
        work = work.display(),
    );

    debug!(overlay_options = %options, "filesystem: mounting overlay");

    // SECURITY: Mount with nosuid and nodev to prevent privilege escalation
    mount(
        Some("overlay"),
        &merged,
        Some("overlay"),
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some(options.as_str()),
    )
    .map_err(|source| {
        FilesystemError::OverlayMount(format!("mount overlay -> {}: {source}", merged.display()))
    })
    .with_context(|| "overlay mount failed")?;

    info!(merged = %merged.display(), "filesystem: overlay mounted");
    Ok(merged)
}

// ---------------------------------------------------------------------------
// pivot_root (child process)
// ---------------------------------------------------------------------------

/// Switch the container's root filesystem using `pivot_root`.
///
/// This must be called **inside the cloned child process** after the overlay
/// has been set up. It performs the following steps:
///
/// 1. **`MS_REC | MS_PRIVATE`** — remount `/` as private so that mount events
///    are no longer propagated to the parent namespace. This is required before
///    `pivot_root(2)`: after `CLONE_NEWNS` the child inherits the parent's
///    shared mount propagation, and `pivot_root` fails with `EINVAL` unless the
///    new root resides on a mount point that is not shared with any peer.
/// 2. Bind-mount `new_root` onto itself to make it a discrete mount point.
/// 3. Mount `proc`, `sysfs`, and `devtmpfs` inside `new_root`.
/// 4. Create `{new_root}/.put_old/` as the landing directory for the old root.
/// 5. Call `pivot_root(new_root, put_old)`.
/// 6. `chdir("/")`, then lazy-unmount and remove `/.put_old/`.
///
/// # Security
///
/// `sysfs` is mounted read-only (`MS_RDONLY`) to prevent cgroup escape via
/// the kernel's sysfs interface. `proc`, `sysfs`, and `devtmpfs` are all
/// mounted with `MS_NOSUID | MS_NODEV | MS_NOEXEC` where applicable to prevent
/// privilege escalation.
pub fn pivot_root_to(new_root: &Path) -> anyhow::Result<()> {
    debug!(new_root = ?new_root, "pivot_root: starting");

    // Disconnect this mount namespace from the parent's propagation tree.
    // After CLONE_NEWNS the child inherits shared mounts; pivot_root(2)
    // requires the new root to be on a different mount than the current
    // root, which fails (EINVAL) if the tree is still shared.
    mount(
        None::<&str>,
        "/",
        None::<&str>,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .map_err(|source| FilesystemError::Mount {
        fs: "private".into(),
        target: "/".into(),
        source,
    })?;

    // Bind-mount new_root onto itself to make it a discrete mount point.
    mount(
        Some(new_root),
        new_root,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    )
    .map_err(|source| FilesystemError::Mount {
        fs: "bind".into(),
        target: new_root.display().to_string(),
        source,
    })?;

    // Mount proc inside new_root.
    // SECURITY: Mount with nosuid, nodev, noexec flags
    let proc_dir = new_root.join("proc");
    fs::create_dir_all(&proc_dir).ok();
    mount(
        Some("proc"),
        &proc_dir,
        Some("proc"),
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
        None::<&str>,
    )
    .map_err(|source| FilesystemError::Mount {
        fs: "proc".into(),
        target: proc_dir.display().to_string(),
        source,
    })?;

    // Mount sysfs inside new_root.
    // SECURITY: Mount read-only with nosuid, nodev, noexec to prevent cgroup escape
    let sys_dir = new_root.join("sys");
    fs::create_dir_all(&sys_dir).ok();
    mount(
        Some("sysfs"),
        &sys_dir,
        Some("sysfs"),
        MsFlags::MS_RDONLY | MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
        None::<&str>,
    )
    .map_err(|source| FilesystemError::Mount {
        fs: "sysfs".into(),
        target: sys_dir.display().to_string(),
        source,
    })?;

    // Mount devtmpfs inside new_root.
    // SECURITY: Mount with nosuid and noexec to prevent privilege escalation
    let dev_dir = new_root.join("dev");
    fs::create_dir_all(&dev_dir).ok();
    mount(
        Some("devtmpfs"),
        &dev_dir,
        Some("devtmpfs"),
        MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC,
        None::<&str>,
    )
    .map_err(|source| FilesystemError::Mount {
        fs: "devtmpfs".into(),
        target: dev_dir.display().to_string(),
        source,
    })?;

    // Create the put_old directory for the old root.
    let put_old = new_root.join(".put_old");
    fs::create_dir_all(&put_old).map_err(|source| FilesystemError::CreateDir {
        path: put_old.display().to_string(),
        source,
    })?;

    // pivot_root(2)
    nix::unistd::pivot_root(new_root, &put_old).map_err(|e| {
        FilesystemError::PivotRoot(format!(
            "pivot_root({}, {}) failed: {e}",
            new_root.display(),
            put_old.display()
        ))
    })?;

    // After pivot_root the old root is at /.put_old.
    nix::unistd::chdir("/").map_err(|e| {
        FilesystemError::PivotRoot(format!("chdir('/') after pivot_root failed: {e}"))
    })?;

    // Unmount the old root filesystem.
    umount2("/.put_old", MntFlags::MNT_DETACH).map_err(|source| FilesystemError::Umount {
        target: "/.put_old".into(),
        source,
    })?;

    fs::remove_dir("/.put_old").ok(); // best-effort

    info!(new_root = ?new_root, "pivot_root: complete");
    Ok(())
}

// ---------------------------------------------------------------------------
// Bind mount setup (child process, inside new mount namespace)
// ---------------------------------------------------------------------------

/// Apply host-path bind mounts into the container rootfs.
///
/// Must be called inside the child process's new mount namespace, after
/// `setup_overlay` but before `pivot_root_to`. Each `BindMount` is
/// applied as an `MS_BIND | MS_REC` mount from `host_path` to
/// `rootfs/container_path`. If `read_only`, a remount with `MS_RDONLY` is
/// applied immediately after.
///
/// On any failure the already-applied mounts are cleaned up (best-effort)
/// before returning the error.
pub fn apply_bind_mounts(
    mounts: &[minibox_core::domain::BindMount],
    rootfs: &Path,
) -> anyhow::Result<()> {
    for (i, m) in mounts.iter().enumerate() {
        if let Err(e) = apply_one_bind_mount(m, rootfs) {
            // Best-effort cleanup of already-applied mounts.
            unmount_bind_mounts(&mounts[..i], rootfs);
            return Err(e);
        }
    }
    Ok(())
}

fn apply_one_bind_mount(m: &minibox_core::domain::BindMount, rootfs: &Path) -> anyhow::Result<()> {
    use anyhow::Context as _;

    // Canonicalize host path — fails fast if the path does not exist.
    let host_canonical = m.host_path.canonicalize().with_context(|| {
        format!(
            "bind mount source {:?} does not exist or is not accessible",
            m.host_path
        )
    })?;

    // Strip leading "/" from container_path so it can be joined to rootfs.
    let container_rel = m
        .container_path
        .strip_prefix("/")
        .unwrap_or(&m.container_path);

    // Reject container_path with ".." components before any filesystem access.
    if has_parent_dir_component(container_rel) {
        anyhow::bail!(
            "path traversal attempt: bind mount container_path contains '..' component: {:?}",
            m.container_path
        );
    }

    let target = rootfs.join(container_rel);

    // Create the mount target if it does not exist.
    if !target.exists() {
        if host_canonical.is_dir() {
            fs::create_dir_all(&target).with_context(|| {
                format!("failed to create bind mount target directory {:?}", target)
            })?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create parent for bind mount target {:?}", target)
                })?;
            }
            fs::write(&target, b"")
                .with_context(|| format!("failed to create bind mount target file {:?}", target))?;
        }
    }

    // Verify the resolved target stays within rootfs (guards against symlink-based
    // traversal through an existing container layer before pivot_root).
    let canonical_rootfs = rootfs
        .canonicalize()
        .with_context(|| format!("failed to canonicalize rootfs {:?}", rootfs))?;
    let canonical_target = target
        .canonicalize()
        .with_context(|| format!("failed to canonicalize bind mount target {:?}", target))?;
    if !canonical_target.starts_with(&canonical_rootfs) {
        anyhow::bail!(
            "path traversal attempt: bind mount target {:?} escapes rootfs {:?}",
            m.container_path,
            rootfs
        );
    }

    // Apply the bind mount.
    mount(
        Some(host_canonical.as_path()),
        canonical_target.as_path(),
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    )
    .map_err(|source| FilesystemError::Mount {
        fs: "bind".into(),
        target: target.display().to_string(),
        source,
    })
    .with_context(|| format!("bind mount {:?} -> {:?} failed", host_canonical, target))?;

    if m.read_only {
        mount(
            None::<&str>,
            canonical_target.as_path(),
            None::<&str>,
            MsFlags::MS_BIND | MsFlags::MS_RDONLY | MsFlags::MS_REMOUNT,
            None::<&str>,
        )
        .map_err(|source| FilesystemError::Mount {
            fs: "bind-ro-remount".into(),
            target: target.display().to_string(),
            source,
        })
        .with_context(|| format!("read-only remount of bind mount {:?} failed", target))?;
    }

    debug!(
        host_path = %host_canonical.display(),
        container_path = %m.container_path.display(),
        read_only = m.read_only,
        "filesystem: bind mount applied"
    );
    Ok(())
}

/// Unmount bind mounts in reverse order. Best-effort: logs warnings on failure.
///
/// Called automatically by `apply_bind_mounts` on partial failure, and
/// should be called by the parent process in cleanup (before `cleanup_mounts`).
pub fn cleanup_bind_mounts(mounts: &[minibox_core::domain::BindMount], rootfs: &Path) {
    unmount_bind_mounts(mounts, rootfs);
}

fn unmount_bind_mounts(mounts: &[minibox_core::domain::BindMount], rootfs: &Path) {
    for m in mounts.iter().rev() {
        let container_rel = m
            .container_path
            .strip_prefix("/")
            .unwrap_or(&m.container_path);
        let target = rootfs.join(container_rel);
        if let Err(e) = umount2(target.as_path(), MntFlags::MNT_DETACH) {
            warn!(
                target = %target.display(),
                error = %e,
                "filesystem: bind mount cleanup failed (best-effort)"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Cleanup (parent process, post container exit)
// ---------------------------------------------------------------------------

/// Unmount the overlay and clean up per-container directories.
pub fn cleanup_mounts(container_dir: &Path) -> anyhow::Result<()> {
    let merged = container_dir.join("merged");
    if merged.exists() {
        debug!(merged = %merged.display(), "filesystem: unmounting overlay");
        if let Err(e) = umount2(&merged, MntFlags::MNT_DETACH) {
            warn!(
                merged = %merged.display(),
                error = %e,
                "filesystem: failed to unmount overlay"
            );
        }
    }

    // Remove the entire per-container directory tree.
    if container_dir.exists() {
        fs::remove_dir_all(container_dir).map_err(|source| FilesystemError::Cleanup {
            path: container_dir.display().to_string(),
            source,
        })?;
    }

    info!(container_dir = %container_dir.display(), "filesystem: container mounts cleaned up");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ---------------------------------------------------------------------------
    // has_parent_dir_component
    // ---------------------------------------------------------------------------

    #[test]
    fn detects_dotdot_at_start() {
        assert!(has_parent_dir_component(Path::new("../escape")));
    }

    #[test]
    fn detects_dotdot_in_middle() {
        assert!(has_parent_dir_component(Path::new("foo/../bar")));
    }

    #[test]
    fn detects_dotdot_multi_hop() {
        assert!(has_parent_dir_component(Path::new("a/b/../../etc/passwd")));
    }

    #[test]
    fn clean_relative_path_passes() {
        assert!(!has_parent_dir_component(Path::new("foo/bar/baz")));
    }

    #[test]
    fn single_dot_passes() {
        assert!(!has_parent_dir_component(Path::new(".")));
    }

    #[test]
    fn absolute_path_without_dotdot_passes() {
        // has_parent_dir_component only checks for ParentDir, not absolute
        assert!(!has_parent_dir_component(Path::new("/absolute/path")));
    }

    // ---------------------------------------------------------------------------
    // validate_layer_path
    // ---------------------------------------------------------------------------

    #[test]
    fn dotdot_at_start_rejected() {
        let base = TempDir::new().unwrap();
        let err = validate_layer_path(Path::new("../escape"), base.path()).unwrap_err();
        assert!(
            err.to_string().contains("path traversal") || err.to_string().contains(".."),
            "expected path traversal error, got: {err}"
        );
    }

    #[test]
    fn dotdot_in_middle_rejected() {
        let base = TempDir::new().unwrap();
        let err = validate_layer_path(Path::new("foo/../bar"), base.path()).unwrap_err();
        assert!(
            err.to_string().contains("path traversal") || err.to_string().contains(".."),
            "expected path traversal error, got: {err}"
        );
    }

    #[test]
    fn path_outside_base_rejected() {
        let base = TempDir::new().unwrap();
        let other = TempDir::new().unwrap();
        // other.path() is a real dir but not under base
        let err = validate_layer_path(other.path(), base.path()).unwrap_err();
        assert!(
            err.to_string().contains("path traversal") || err.to_string().contains("outside"),
            "expected outside-base error, got: {err}"
        );
    }

    #[test]
    fn valid_subdir_within_base_accepted() {
        let base = TempDir::new().unwrap();
        let subdir = base.path().join("images").join("alpine").join("layer1");
        std::fs::create_dir_all(&subdir).unwrap();
        validate_layer_path(&subdir, base.path()).unwrap();
    }

    #[test]
    fn deeply_nested_subdir_accepted() {
        let base = TempDir::new().unwrap();
        let subdir = base.path().join("a").join("b").join("c").join("d");
        std::fs::create_dir_all(&subdir).unwrap();
        validate_layer_path(&subdir, base.path()).unwrap();
    }

    #[test]
    fn path_equal_to_base_accepted() {
        let base = TempDir::new().unwrap();
        // The base dir itself counts as within base
        validate_layer_path(base.path(), base.path()).unwrap();
    }

    // ── apply_bind_mounts ────────────────────────────────────────────────────
    // These tests require Linux (MS_BIND is Linux-only) and root.
    // Run with: sudo cargo test -p linuxbox container::filesystem::tests

    #[cfg(target_os = "linux")]
    mod bind_mount_tests {
        use super::*;
        use minibox_core::domain::BindMount;
        use std::path::PathBuf;
        use tempfile::TempDir;

        #[test]
        fn apply_bind_mounts_mounts_directory() {
            if unsafe { libc::geteuid() } != 0 {
                return;
            }

            let host_dir = TempDir::new().unwrap();
            let rootfs = TempDir::new().unwrap();

            // Write a sentinel file into the host directory.
            std::fs::write(host_dir.path().join("sentinel.txt"), b"hello").unwrap();

            let mounts = vec![BindMount {
                host_path: host_dir.path().to_path_buf(),
                container_path: PathBuf::from("/data"),
                read_only: false,
            }];

            apply_bind_mounts(&mounts, rootfs.path()).unwrap();

            // The sentinel should be visible at rootfs/data/sentinel.txt
            let sentinel = rootfs.path().join("data").join("sentinel.txt");
            assert!(sentinel.exists(), "bind mount not visible at target");

            cleanup_bind_mounts(&mounts, rootfs.path());
        }

        #[test]
        fn apply_bind_mounts_read_only() {
            if unsafe { libc::geteuid() } != 0 {
                return;
            }

            let host_dir = TempDir::new().unwrap();
            let rootfs = TempDir::new().unwrap();

            let mounts = vec![BindMount {
                host_path: host_dir.path().to_path_buf(),
                container_path: PathBuf::from("/ro"),
                read_only: true,
            }];

            apply_bind_mounts(&mounts, rootfs.path()).unwrap();

            // Writing to the read-only mount should fail.
            let result = std::fs::write(rootfs.path().join("ro").join("test.txt"), b"fail");
            assert!(result.is_err(), "expected write to read-only mount to fail");

            cleanup_bind_mounts(&mounts, rootfs.path());
        }

        #[test]
        fn apply_bind_mounts_nonexistent_host_path_fails() {
            let rootfs = TempDir::new().unwrap();
            let mounts = vec![BindMount {
                host_path: PathBuf::from("/nonexistent/path/that/does/not/exist"),
                container_path: PathBuf::from("/data"),
                read_only: false,
            }];
            let result = apply_bind_mounts(&mounts, rootfs.path());
            assert!(result.is_err());
        }

        #[test]
        fn apply_bind_mounts_rejects_dotdot_in_container_path() {
            let host_dir = TempDir::new().unwrap();
            let rootfs = TempDir::new().unwrap();
            // "/../../../etc" after strip_prefix("/") → "../../../etc" — must be rejected.
            let mounts = vec![BindMount {
                host_path: host_dir.path().to_path_buf(),
                container_path: PathBuf::from("/../../../etc"),
                read_only: false,
            }];
            let result = apply_bind_mounts(&mounts, rootfs.path());
            assert!(result.is_err());
            let msg = format!("{:#}", result.unwrap_err());
            assert!(
                msg.contains("path traversal"),
                "expected 'path traversal' in error, got: {msg}"
            );
        }

        #[test]
        fn apply_bind_mounts_rejects_relative_dotdot_container_path() {
            let host_dir = TempDir::new().unwrap();
            let rootfs = TempDir::new().unwrap();
            let mounts = vec![BindMount {
                host_path: host_dir.path().to_path_buf(),
                container_path: PathBuf::from("../escape"),
                read_only: false,
            }];
            let result = apply_bind_mounts(&mounts, rootfs.path());
            assert!(result.is_err());
            let msg = format!("{:#}", result.unwrap_err());
            assert!(
                msg.contains("path traversal"),
                "expected 'path traversal' in error, got: {msg}"
            );
        }

        #[test]
        fn apply_bind_mounts_creates_target_dir() {
            if unsafe { libc::geteuid() } != 0 {
                return;
            }

            let host_dir = TempDir::new().unwrap();
            let rootfs = TempDir::new().unwrap();

            let mounts = vec![BindMount {
                host_path: host_dir.path().to_path_buf(),
                container_path: PathBuf::from("/nested/dir/target"),
                read_only: false,
            }];

            // Target dir does not exist yet — apply_bind_mounts must create it.
            apply_bind_mounts(&mounts, rootfs.path()).unwrap();
            assert!(rootfs.path().join("nested/dir/target").is_dir());

            cleanup_bind_mounts(&mounts, rootfs.path());
        }
    }
}
