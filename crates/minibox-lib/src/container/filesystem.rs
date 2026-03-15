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
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

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
pub fn setup_overlay(image_layers: &[PathBuf], container_dir: &Path) -> anyhow::Result<PathBuf> {
    let merged = container_dir.join("merged");
    let upper = container_dir.join("upper");
    let work = container_dir.join("work");

    for dir in [&merged, &upper, &work] {
        fs::create_dir_all(dir).map_err(|source| FilesystemError::CreateDir {
            path: dir.display().to_string(),
            source,
        })?;
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

    mount(
        Some("overlay"),
        &merged,
        Some("overlay"),
        MsFlags::empty(),
        Some(options.as_str()),
    )
    .map_err(|source| {
        FilesystemError::OverlayMount(format!(
            "mount overlay -> {}: {source}",
            merged.display()
        ))
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
    let proc_dir = new_root.join("proc");
    fs::create_dir_all(&proc_dir).ok();
    mount(
        Some("proc"),
        &proc_dir,
        Some("proc"),
        MsFlags::empty(),
        None::<&str>,
    )
    .map_err(|source| FilesystemError::Mount {
        fs: "proc".into(),
        target: proc_dir.display().to_string(),
        source,
    })?;

    // Mount sysfs inside new_root.
    let sys_dir = new_root.join("sys");
    fs::create_dir_all(&sys_dir).ok();
    mount(
        Some("sysfs"),
        &sys_dir,
        Some("sysfs"),
        MsFlags::empty(),
        None::<&str>,
    )
    .map_err(|source| FilesystemError::Mount {
        fs: "sysfs".into(),
        target: sys_dir.display().to_string(),
        source,
    })?;

    // Mount devtmpfs inside new_root.
    let dev_dir = new_root.join("dev");
    fs::create_dir_all(&dev_dir).ok();
    mount(
        Some("devtmpfs"),
        &dev_dir,
        Some("devtmpfs"),
        MsFlags::empty(),
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
