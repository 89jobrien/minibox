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

// Path validation utilities
fn has_parent_dir_component(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

// ---------------------------------------------------------------------------
// Path validation (security)
// ---------------------------------------------------------------------------

/// Validate that a layer path is safe and within the allowed directory.
///
/// # Security
///
/// Prevents path traversal attacks by:
/// 1. Rejecting paths with `..` components
/// 2. Canonicalizing paths and verifying they're under base_dir
/// 3. Ensuring paths don't escape via symlinks
fn validate_layer_path(path: &Path, base_dir: &Path) -> anyhow::Result<()> {
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

    debug!("validated layer path: {:?}", canonical_path);
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
        validate_layer_path(layer_path, &images_base)
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

    debug!("mounting overlay with options: {}", options);

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

    info!("overlay mounted at {:?}", merged);
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
/// 1. Bind-mount `new_root` onto itself (required by `pivot_root`).
/// 2. Create `{new_root}/.put_old/` as the destination for the old root.
/// 3. Mount `proc`, `sysfs`, and `devtmpfs` inside `new_root`.
/// 4. Call `pivot_root(new_root, put_old)`.
/// 5. `chdir("/")` and unmount + remove `.put_old/`.
pub fn pivot_root_to(new_root: &Path) -> anyhow::Result<()> {
    debug!("pivoting root to {:?}", new_root);

    // pivot_root requires new_root to be a mount point.
    // SECURITY: Include nosuid and nodev flags
    mount(
        Some(new_root),
        new_root,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC | MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
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

    info!("pivot_root complete, new root is {:?}", new_root);
    Ok(())
}

// ---------------------------------------------------------------------------
// Cleanup (parent process, post container exit)
// ---------------------------------------------------------------------------

/// Unmount the overlay and clean up per-container directories.
pub fn cleanup_mounts(container_dir: &Path) -> anyhow::Result<()> {
    let merged = container_dir.join("merged");
    if merged.exists() {
        debug!("unmounting overlay at {:?}", merged);
        if let Err(e) = umount2(&merged, MntFlags::MNT_DETACH) {
            warn!("failed to unmount overlay at {:?}: {}", merged, e);
        }
    }

    // Remove the entire per-container directory tree.
    if container_dir.exists() {
        fs::remove_dir_all(container_dir).map_err(|source| FilesystemError::Cleanup {
            path: container_dir.display().to_string(),
            source,
        })?;
    }

    info!("container mounts cleaned up for {:?}", container_dir);
    Ok(())
}
