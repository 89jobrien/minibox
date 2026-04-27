//! GKE unprivileged adapter suite for running minibox in standard GKE pods.
//!
//! Standard GKE pods lack `CAP_SYS_ADMIN`, blocking `mount()`, `pivot_root()`,
//! `clone()` with namespace flags, overlay FS, and cgroup writes. This module
//! provides three adapters that work within those constraints:
//!
//! - [`NoopLimiter`]: No-op resource limiter (cgroups unavailable)
//! - [`CopyFilesystem`]: Copy-based layer merging (no overlay FS)
//! - [`ProotRuntime`]: proot (ptrace-based) fake chroot runtime
//!
//! # Architecture
//!
//! ```text
//! GKE Pod (unprivileged)
//! ┌─────────────────────────────────────────┐
//! │  miniboxd (non-root)                    │
//! │  ┌───────────────────┐                  │
//! │  │ NoopLimiter       │ (no cgroups)     │
//! │  │ CopyFilesystem    │ (cp, not overlay)│
//! │  │ ProotRuntime      │ (ptrace chroot)  │
//! │  └───────────────────┘                  │
//! └─────────────────────────────────────────┘
//! ```
//!
//! # Selection
//!
//! Activated by setting `MINIBOX_ADAPTER=gke` at daemon startup. The same
//! binary works in both native and GKE modes — no recompilation needed.

use anyhow::{Context, Result};
use async_trait::async_trait;
use minibox_core::domain::{
    ContainerRuntime, ContainerSpawnConfig, ResourceConfig, ResourceLimiter, RootfsLayout,
    RuntimeCapabilities, SpawnResult,
};
use minibox_core::{adapt, as_any};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

// ============================================================================
// NoopLimiter
// ============================================================================

/// No-op resource limiter for environments without cgroups access.
///
/// Returns a sentinel cgroup path that the runtime ignores. All operations
/// are no-ops that return `Ok(())`.
#[derive(Debug, Clone, Copy)]
pub struct NoopLimiter;

#[allow(dead_code)]
impl NoopLimiter {
    pub fn new() -> Self {
        Self
    }
}

impl ResourceLimiter for NoopLimiter {
    fn create(&self, container_id: &str, _config: &ResourceConfig) -> Result<String> {
        debug!("noop limiter: skipping cgroup creation for container {container_id}");
        Ok(format!("noop:{container_id}"))
    }

    fn add_process(&self, container_id: &str, pid: u32) -> Result<()> {
        debug!("noop limiter: skipping add_process({pid}) for container {container_id}");
        Ok(())
    }

    fn cleanup(&self, container_id: &str) -> Result<()> {
        debug!("noop limiter: skipping cleanup for container {container_id}");
        Ok(())
    }
}

// ============================================================================
// CopyFilesystem
// ============================================================================

/// Copy-based filesystem provider for environments without overlay FS support.
///
/// Merges image layers by copying files bottom-to-top (later layers overwrite
/// earlier ones). Preserves file permissions and symlinks. Skips device nodes,
/// named pipes, and sockets with a warning.
#[derive(Debug, Clone, Copy)]
pub struct CopyFilesystem;

#[allow(dead_code)]
impl CopyFilesystem {
    pub fn new() -> Self {
        Self
    }
}

impl minibox_core::domain::RootfsSetup for CopyFilesystem {
    fn setup_rootfs(&self, image_layers: &[PathBuf], container_dir: &Path) -> Result<RootfsLayout> {
        let merged = container_dir.join("merged");
        std::fs::create_dir_all(&merged)
            .with_context(|| format!("creating merged dir {merged:?}"))?;

        debug!(
            "copy filesystem: merging {} layers into {:?}",
            image_layers.len(),
            merged
        );

        // Copy layers bottom-to-top; later layers overwrite earlier files.
        for layer in image_layers {
            if layer.is_dir() {
                copy_dir_into(layer, &merged)
                    .with_context(|| format!("copying layer {layer:?}"))?;
            } else {
                warn!("skipping non-directory layer: {:?}", layer);
            }
        }

        Ok(RootfsLayout {
            merged_dir: merged,
            rootfs_metadata: None,
            source_image_ref: None,
        })
    }

    fn cleanup(&self, container_dir: &Path) -> Result<()> {
        debug!("copy filesystem: removing {:?}", container_dir);
        if container_dir.exists() {
            std::fs::remove_dir_all(container_dir)
                .with_context(|| format!("removing container dir {container_dir:?}"))?;
        }
        Ok(())
    }
}

impl minibox_core::domain::ChildInit for CopyFilesystem {
    fn pivot_root(&self, _new_root: &Path) -> Result<()> {
        // proot handles the fake chroot — nothing to do here.
        debug!("copy filesystem: pivot_root is a no-op (proot handles fake chroot)");
        Ok(())
    }
}

/// Recursively copy the contents of `src` into `dst`, preserving permissions
/// and symlinks. Skips device nodes, named pipes, and sockets.
#[allow(dead_code)]
fn copy_dir_into(src: &Path, dst: &Path) -> Result<()> {
    use walkdir::WalkDir;

    for entry in WalkDir::new(src).min_depth(1) {
        let entry = entry.with_context(|| format!("walking {src:?}"))?;
        let relative = entry.path().strip_prefix(src).context("stripping prefix")?;
        let target = dst.join(relative);

        let ft = entry.file_type();

        if ft.is_dir() {
            std::fs::create_dir_all(&target).with_context(|| format!("creating dir {target:?}"))?;
            // Preserve directory permissions
            let metadata = entry.metadata().context("reading dir metadata")?;
            std::fs::set_permissions(&target, metadata.permissions())
                .with_context(|| format!("setting permissions on {target:?}"))?;
        } else if ft.is_symlink() {
            let link_target = std::fs::read_link(entry.path())
                .with_context(|| format!("reading symlink {:?}", entry.path()))?;
            // Remove existing file/symlink at target before creating new one
            if target.exists() || target.symlink_metadata().is_ok() {
                std::fs::remove_file(&target).ok();
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link_target, &target)
                .with_context(|| format!("creating symlink {target:?} -> {link_target:?}"))?;
            #[cfg(not(unix))]
            std::fs::copy(entry.path(), &target)
                .with_context(|| format!("copying (non-unix symlink) {:?}", entry.path()))?;
        } else if ft.is_file() {
            // Ensure parent directory exists
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &target)
                .with_context(|| format!("copying {:?}", entry.path()))?;
            // Preserve file permissions
            let metadata = entry.metadata().context("reading file metadata")?;
            std::fs::set_permissions(&target, metadata.permissions())
                .with_context(|| format!("setting permissions on {target:?}"))?;
        } else {
            // Device nodes, named pipes, sockets — skip with warning
            warn!(
                "copy filesystem: skipping special file {:?} (type: {:?})",
                entry.path(),
                ft
            );
        }
    }

    Ok(())
}

// ============================================================================
// ProotRuntime
// ============================================================================

/// proot-based container runtime for unprivileged environments.
///
/// Uses [proot](https://proot-me.github.io/) to provide fake chroot via
/// ptrace syscall interception. No kernel privileges required.
///
/// # proot invocation
///
/// ```text
/// proot -r <rootfs> -0 -b /proc:/proc -b /dev:/dev -w / <command> [args...]
/// ```
///
/// - `-r`: Set the new root filesystem
/// - `-0`: Fake root (UID 0) inside the container
/// - `-b`: Bind-mount host paths into the container
/// - `-w /`: Set working directory to /
#[derive(Debug, Clone)]
pub struct ProotRuntime {
    proot_path: PathBuf,
}

#[allow(dead_code)]
impl ProotRuntime {
    /// Create a new proot runtime with an explicit binary path.
    ///
    /// Returns an error if the binary does not exist.
    pub fn new(proot_path: impl Into<PathBuf>) -> Result<Self> {
        let proot_path = proot_path.into();
        if !proot_path.exists() {
            anyhow::bail!("proot binary not found at {proot_path:?}");
        }
        Ok(Self { proot_path })
    }

    /// Create a proot runtime from environment.
    ///
    /// Checks `MINIBOX_PROOT_PATH` first, then searches `PATH` for `proot`.
    pub fn from_env() -> Result<Self> {
        if let Ok(path) = std::env::var("MINIBOX_PROOT_PATH") {
            return Self::new(&path)
                .with_context(|| format!("MINIBOX_PROOT_PATH points to invalid binary {path:?}"));
        }

        // Search PATH for proot
        if let Ok(output) = std::process::Command::new("which").arg("proot").output()
            && output.status.success()
        {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Self::new(path);
            }
        }

        anyhow::bail!("proot not found: set MINIBOX_PROOT_PATH or install proot in PATH")
    }
}

#[async_trait]
impl ContainerRuntime for ProotRuntime {
    fn capabilities(&self) -> RuntimeCapabilities {
        // proot runs unprivileged: no real namespaces, no cgroups, no overlay
        RuntimeCapabilities {
            supports_user_namespaces: false,
            supports_cgroups_v2: false,
            supports_overlay_fs: false,
            supports_network_isolation: false,
            max_containers: None,
        }
    }

    async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<SpawnResult> {
        debug!(
            "proot runtime: spawning command={} in rootfs={:?}",
            config.command, config.rootfs
        );

        let proot_path = self.proot_path.clone();
        let rootfs = config.rootfs.clone();
        let command = config.command.clone();
        let args = config.args.clone();
        let env = config.env.clone();

        let pid = tokio::task::spawn_blocking(move || -> Result<u32> {
            let mut cmd = std::process::Command::new(&proot_path);

            // proot flags: fake root, bind /proc and /dev, set working dir
            cmd.arg("-r")
                .arg(&rootfs)
                .arg("-0")
                .arg("-b")
                .arg("/proc:/proc")
                .arg("-b")
                .arg("/dev:/dev")
                .arg("-w")
                .arg("/")
                .arg(&command);

            // Append command arguments
            for arg in &args {
                cmd.arg(arg);
            }

            // Clear inherited env, set only container env vars
            cmd.env_clear();
            for var in &env {
                if let Some((key, value)) = var.split_once('=') {
                    cmd.env(key, value);
                }
            }

            let child = cmd
                .spawn()
                .with_context(|| format!("spawning proot at {proot_path:?}"))?;

            let pid = child.id();

            // Prevent Child::drop from sending SIGKILL — the daemon's
            // waitpid loop in handler.rs is the reaper.
            std::mem::forget(child);

            Ok(pid)
        })
        .await??;

        debug!("proot runtime: spawned with PID {}", pid);
        Ok(SpawnResult {
            runtime_id: None,
            pid,
            output_reader: None,
        })
    }
}

// ============================================================================
// Macro-generated implementations
// ============================================================================

adapt!(NoopLimiter, CopyFilesystem);
as_any!(ProotRuntime);

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::domain::{ChildInit, RootfsSetup};
    use tempfile::TempDir;

    // -- NoopLimiter tests --------------------------------------------------

    #[test]
    fn noop_limiter_create_returns_sentinel() {
        let limiter = NoopLimiter::new();
        let config = ResourceConfig::default();
        let path = limiter.create("test-container", &config).unwrap();
        assert_eq!(path, "noop:test-container");
    }

    #[test]
    fn noop_limiter_add_process_succeeds() {
        let limiter = NoopLimiter::new();
        assert!(limiter.add_process("test-container", 1234).is_ok());
    }

    #[test]
    fn noop_limiter_cleanup_succeeds() {
        let limiter = NoopLimiter::new();
        assert!(limiter.cleanup("test-container").is_ok());
    }

    #[test]
    fn noop_limiter_default() {
        let limiter = NoopLimiter;
        let _ = limiter;
    }

    // -- CopyFilesystem tests -----------------------------------------------

    #[test]
    fn copy_filesystem_setup_creates_merged_dir() {
        let dir = TempDir::new().unwrap();
        let container_dir = dir.path().join("container");
        std::fs::create_dir_all(&container_dir).unwrap();

        let fs = CopyFilesystem::new();
        let merged = fs.setup_rootfs(&[], &container_dir).unwrap();

        assert!(merged.merged_dir.exists());
        assert!(merged.merged_dir.ends_with("merged"));
    }

    #[test]
    fn copy_filesystem_merges_layers_bottom_to_top() {
        let dir = TempDir::new().unwrap();

        // Create two layers
        let layer1 = dir.path().join("layer1");
        let layer2 = dir.path().join("layer2");
        std::fs::create_dir_all(layer1.join("etc")).unwrap();
        std::fs::create_dir_all(layer2.join("etc")).unwrap();

        // layer1 has a file
        std::fs::write(layer1.join("etc/hostname"), "layer1-host").unwrap();
        std::fs::write(layer1.join("etc/base"), "from-layer1").unwrap();

        // layer2 overwrites one file, adds another
        std::fs::write(layer2.join("etc/hostname"), "layer2-host").unwrap();
        std::fs::write(layer2.join("etc/extra"), "from-layer2").unwrap();

        let container_dir = dir.path().join("container");
        let fs = CopyFilesystem::new();
        let merged = fs.setup_rootfs(&[layer1, layer2], &container_dir).unwrap();

        // layer2 should overwrite layer1's hostname
        assert_eq!(
            std::fs::read_to_string(merged.merged_dir.join("etc/hostname")).unwrap(),
            "layer2-host"
        );
        // layer1's base file should be preserved
        assert_eq!(
            std::fs::read_to_string(merged.merged_dir.join("etc/base")).unwrap(),
            "from-layer1"
        );
        // layer2's extra file should be present
        assert_eq!(
            std::fs::read_to_string(merged.merged_dir.join("etc/extra")).unwrap(),
            "from-layer2"
        );
    }

    #[test]
    fn copy_filesystem_preserves_permissions() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let dir = TempDir::new().unwrap();
            let layer = dir.path().join("layer");
            std::fs::create_dir_all(layer.join("bin")).unwrap();

            let script = layer.join("bin/run.sh");
            std::fs::write(&script, "#!/bin/sh\necho hello").unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

            let container_dir = dir.path().join("container");
            let fs = CopyFilesystem::new();
            let merged = fs.setup_rootfs(&[layer], &container_dir).unwrap();

            let metadata = std::fs::metadata(merged.merged_dir.join("bin/run.sh")).unwrap();
            let mode = metadata.permissions().mode() & 0o777;
            assert_eq!(mode, 0o755);
        }
    }

    #[cfg(unix)]
    #[test]
    fn copy_filesystem_preserves_symlinks() {
        let dir = TempDir::new().unwrap();
        let layer = dir.path().join("layer");
        std::fs::create_dir_all(layer.join("usr/bin")).unwrap();
        std::fs::write(layer.join("usr/bin/python3"), "fake-binary").unwrap();
        std::os::unix::fs::symlink("python3", layer.join("usr/bin/python")).unwrap();

        let container_dir = dir.path().join("container");
        let fs = CopyFilesystem::new();
        let merged = fs.setup_rootfs(&[layer], &container_dir).unwrap();

        let link = std::fs::read_link(merged.merged_dir.join("usr/bin/python")).unwrap();
        assert_eq!(link, PathBuf::from("python3"));
    }

    #[test]
    fn copy_filesystem_pivot_root_is_noop() {
        let fs = CopyFilesystem::new();
        assert!(fs.pivot_root(Path::new("/nonexistent")).is_ok());
    }

    #[test]
    fn copy_filesystem_cleanup_removes_dir() {
        let dir = TempDir::new().unwrap();
        let container_dir = dir.path().join("container");
        std::fs::create_dir_all(container_dir.join("merged/etc")).unwrap();
        std::fs::write(container_dir.join("merged/etc/test"), "data").unwrap();

        let fs = CopyFilesystem::new();
        fs.cleanup(&container_dir).unwrap();
        assert!(!container_dir.exists());
    }

    #[test]
    fn copy_filesystem_cleanup_nonexistent_is_ok() {
        let fs = CopyFilesystem::new();
        assert!(
            fs.cleanup(Path::new("/tmp/nonexistent-minibox-test"))
                .is_ok()
        );
    }

    #[test]
    fn copy_filesystem_default() {
        let fs = CopyFilesystem;
        let _ = fs;
    }

    // -- ProotRuntime tests -------------------------------------------------

    #[test]
    fn proot_runtime_rejects_missing_binary() {
        let result = ProotRuntime::new("/nonexistent/proot");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("proot binary not found")
        );
    }

    #[test]
    fn proot_runtime_accepts_existing_binary() {
        // /bin/sh always exists on Unix
        #[cfg(unix)]
        {
            let runtime = ProotRuntime::new("/bin/sh");
            assert!(runtime.is_ok());
        }
    }

    // -- ProotRuntime::capabilities() tests ---------------------------------

    #[test]
    fn proot_runtime_capabilities_all_false() {
        #[cfg(unix)]
        {
            let runtime = ProotRuntime::new("/bin/sh").unwrap();
            let caps = runtime.capabilities();
            assert!(!caps.supports_user_namespaces);
            assert!(!caps.supports_cgroups_v2);
            assert!(!caps.supports_overlay_fs);
            assert!(!caps.supports_network_isolation);
            assert!(caps.max_containers.is_none());
        }
    }

    // -- ProotRuntime::from_env() tests -------------------------------------

    use std::sync::Mutex;
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn proot_runtime_from_env_uses_env_var() {
        let _guard = ENV_MUTEX.lock().unwrap();
        // SAFETY: serialized by ENV_MUTEX; no other thread reads MINIBOX_PROOT_PATH concurrently.
        unsafe { std::env::set_var("MINIBOX_PROOT_PATH", "/bin/sh") };
        let result = ProotRuntime::from_env();
        // SAFETY: same mutex guard; restoring env before any other test runs.
        unsafe { std::env::remove_var("MINIBOX_PROOT_PATH") };
        assert!(result.is_ok(), "expected ProotRuntime, got {result:?}");
    }

    #[test]
    fn proot_runtime_from_env_fails_when_proot_not_found() {
        let _guard = ENV_MUTEX.lock().unwrap();
        // SAFETY: serialized by ENV_MUTEX; no other thread reads MINIBOX_PROOT_PATH concurrently.
        unsafe { std::env::remove_var("MINIBOX_PROOT_PATH") };

        // Only run this assertion when proot is genuinely absent; skip otherwise.
        let proot_on_path = std::process::Command::new("which")
            .arg("proot")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if proot_on_path {
            // proot is installed — we cannot test the failure branch; skip.
            return;
        }

        let result = ProotRuntime::from_env();
        assert!(result.is_err(), "expected Err when proot not found");
        assert!(
            result.unwrap_err().to_string().contains("proot not found"),
            "error message should mention 'proot not found'"
        );
    }

    // -- CopyFilesystem: non-directory layer skip path ----------------------

    #[test]
    fn copy_filesystem_skips_non_directory_layer() {
        let dir = TempDir::new().unwrap();

        // A regular file masquerading as a "layer" path
        let fake_layer = dir.path().join("fake_layer.tar");
        std::fs::write(&fake_layer, b"not a dir").unwrap();

        let container_dir = dir.path().join("container");
        let fs = CopyFilesystem::new();

        // Non-directory layers are skipped with warn! — setup_rootfs should still succeed.
        let result = fs.setup_rootfs(&[fake_layer], &container_dir);
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        assert!(
            result.unwrap().merged_dir.exists(),
            "merged dir should exist even after skipping bad layer"
        );
    }
}
