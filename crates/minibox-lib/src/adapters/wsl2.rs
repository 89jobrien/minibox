//! Windows Subsystem for Linux 2 (WSL2) adapter for cross-platform support.
//!
//! This adapter delegates container operations to a Linux environment running
//! in WSL2, enabling Windows users to run minibox containers without a
//! separate VM manager. The adapter communicates with a `minibox-wsl-helper`
//! binary compiled for Linux and installed inside the WSL2 distribution.
//!
//! These adapters implement [`ContainerRuntime`], [`FilesystemProvider`], and
//! [`ResourceLimiter`]. Image pulling is handled by [`DockerHubRegistry`] on
//! the Windows side (no WSL2-specific registry adapter is needed).
//!
//! These adapters are **library-only** — they are not wired into `miniboxd`
//! and cannot be selected via `MINIBOX_ADAPTER`.
//!
//! # Architecture
//!
//! ```text
//! Windows Host
//! ┌────────────────────────────────────────┐
//! │  miniboxd (Windows)                    │
//! │  ┌──────────────┐                      │
//! │  │ Wsl2Runtime   │                      │
//! │  │ Wsl2Filesystem│  ---wsl.exe--->      │
//! │  │ Wsl2Limiter   │                      │
//! │  └──────────────┘                      │
//! └────────────────────────────────────────┘
//!                 │
//!                 ▼
//! WSL2 (Linux VM)
//! ┌────────────────────────────────────────┐
//! │  minibox-wsl-helper (Linux binary)     │
//! │  ┌──────────────┐                      │
//! │  │ Real Linux   │                      │
//! │  │ Namespaces   │                      │
//! │  │ cgroups v2   │                      │
//! │  │ overlayfs    │                      │
//! │  └──────────────┘                      │
//! └────────────────────────────────────────┘
//! ```
//!
//! # Requirements
//!
//! - Windows 10/11 with WSL2 enabled
//! - WSL2 Linux distribution (Ubuntu 20.04+ recommended)
//! - `minibox-wsl-helper` binary installed in the WSL2 distribution
//!
//! # Setup
//!
//! ```bash
//! # Install WSL2
//! wsl --install
//!
//! # Build the helper inside WSL2
//! wsl -d Ubuntu cargo build --release -p minibox-wsl-helper
//!
//! # Install the helper
//! wsl -d Ubuntu sudo cp target/release/minibox-wsl-helper /usr/local/bin/
//! ```

use crate::{
    as_any,
    domain::{
        ContainerRuntime, ContainerSpawnConfig, FilesystemProvider, ResourceConfig,
        ResourceLimiter, RuntimeCapabilities, SpawnResult,
    },
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::debug;

/// WSL2-based implementation of [`ContainerRuntime`].
///
/// Delegates container spawning to `minibox-wsl-helper spawn` running inside
/// the named WSL2 distribution. Windows paths are translated to WSL2 paths via
/// `wslpath` before being sent to the helper.
#[derive(Debug, Clone)]
pub struct Wsl2Runtime {
    /// WSL2 distribution name passed to `wsl.exe -d` (e.g. `"Ubuntu"`, `"Debian"`).
    distro: String,
    /// Absolute path to `minibox-wsl-helper` inside the WSL2 filesystem
    /// (e.g. `"/usr/local/bin/minibox-wsl-helper"`).
    helper_path: String,
}

impl Wsl2Runtime {
    /// Create a new WSL2 runtime adapter.
    ///
    /// # Arguments
    ///
    /// * `distro` - WSL2 distribution name (run `wsl -l` to list installed distributions).
    /// * `helper_path` - Path to `minibox-wsl-helper` inside the WSL2 filesystem.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let runtime = Wsl2Runtime::new("Ubuntu", "/usr/local/bin/minibox-wsl-helper");
    /// ```
    pub fn new(distro: impl Into<String>, helper_path: impl Into<String>) -> Self {
        Self {
            distro: distro.into(),
            helper_path: helper_path.into(),
        }
    }

    /// Execute a command inside the WSL2 distribution and return its stdout.
    ///
    /// Runs `wsl.exe -d <distro> -- <args…>` and captures stdout as UTF-8.
    ///
    /// # Errors
    ///
    /// Returns an error if `wsl.exe` cannot be spawned, if the command exits
    /// with a non-zero status, or if stdout is not valid UTF-8.
    fn wsl_exec(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("wsl.exe")
            .arg("-d")
            .arg(&self.distro)
            .arg("--")
            .args(args)
            .output()
            .context("failed to execute wsl.exe")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("WSL command failed: {stderr}");
        }

        String::from_utf8(output.stdout).context("invalid UTF-8 from WSL")
    }

    /// Convert a Windows-style path to its WSL2-accessible equivalent.
    ///
    /// Delegates to `wslpath -u` inside the WSL2 distribution. For example,
    /// `C:\Users\test\file.txt` becomes `/mnt/c/Users/test/file.txt`.
    ///
    /// # Errors
    ///
    /// Returns an error if the path contains non-UTF-8 characters, if
    /// `wsl.exe` cannot be spawned, or if `wslpath` fails.
    fn windows_to_wsl_path(&self, windows_path: &Path) -> Result<String> {
        let path_str = windows_path.to_str().context("invalid path")?;

        let wsl_path = self
            .wsl_exec(&["wslpath", "-u", path_str])
            .context("failed to convert Windows path to WSL path")?;

        Ok(wsl_path.trim().to_string())
    }
}

#[async_trait]
impl ContainerRuntime for Wsl2Runtime {
    /// Return the runtime capabilities advertised by this adapter.
    ///
    /// WSL2 delegates to a full Linux kernel, so all namespace and cgroup
    /// features are available inside the distribution.
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            supports_user_namespaces: true,
            supports_cgroups_v2: true,
            supports_overlay_fs: true,
            supports_network_isolation: true,
            max_containers: None,
        }
    }

    /// Spawn a container process in WSL2 and return its Linux PID.
    ///
    /// Translates both `rootfs` and `cgroup_path` from Windows paths to WSL2
    /// paths, serialises the full spawn configuration to JSON, and invokes
    /// `minibox-wsl-helper spawn <json>` inside the distribution via
    /// `tokio::task::spawn_blocking` (required because `wsl.exe` is a
    /// blocking process spawn).
    ///
    /// The `output_reader` field of the returned [`SpawnResult`] is always
    /// `None` — output streaming from WSL2-hosted containers is not yet
    /// implemented.
    ///
    /// # Errors
    ///
    /// Returns an error if path translation fails, if `wsl.exe` cannot be
    /// spawned, if the helper exits with a non-zero status, or if the JSON
    /// response cannot be parsed.
    async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<SpawnResult> {
        debug!(
            command = %config.command,
            rootfs = ?config.rootfs,
            "wsl2: spawning container"
        );

        // Translate Windows paths to their WSL2 equivalents before sending
        // them to the helper binary running inside the Linux VM.
        let rootfs_wsl = self.windows_to_wsl_path(&config.rootfs)?;
        let cgroup_path_wsl = self.windows_to_wsl_path(&config.cgroup_path)?;

        let spawn_request = WslSpawnRequest {
            rootfs: rootfs_wsl,
            command: config.command.clone(),
            args: config.args.clone(),
            env: config.env.clone(),
            hostname: config.hostname.clone(),
            cgroup_path: cgroup_path_wsl,
        };

        let json = serde_json::to_string(&spawn_request).context("failed to serialize request")?;

        // wsl.exe is a blocking call; run it off the async runtime.
        let distro = self.distro.clone();
        let helper_path = self.helper_path.clone();

        let output = tokio::task::spawn_blocking(move || {
            Command::new("wsl.exe")
                .arg("-d")
                .arg(&distro)
                .arg("--")
                .arg("sudo")
                .arg(&helper_path)
                .arg("spawn")
                .arg(&json)
                .output()
        })
        .await?
        .context("failed to execute WSL helper")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("WSL helper spawn failed: {stderr}");
        }

        let stdout = String::from_utf8(output.stdout).context("invalid UTF-8 from helper")?;
        let response: WslSpawnResponse =
            serde_json::from_str(&stdout).context("failed to parse spawn response")?;

        debug!(pid = response.pid, "wsl2: container spawned");
        Ok(SpawnResult {
            pid: response.pid,
            output_reader: None,
        })
    }
}

/// WSL2-based implementation of [`FilesystemProvider`].
///
/// Delegates overlay filesystem setup and teardown to `minibox-wsl-helper`
/// running inside the named WSL2 distribution.
#[derive(Debug, Clone)]
pub struct Wsl2Filesystem {
    /// Shared runtime handle used for path translation and WSL exec.
    runtime: Wsl2Runtime,
}

impl Wsl2Filesystem {
    /// Create a new WSL2 filesystem adapter.
    ///
    /// # Arguments
    ///
    /// * `distro` - WSL2 distribution name.
    /// * `helper_path` - Path to `minibox-wsl-helper` inside the WSL2 filesystem.
    pub fn new(distro: impl Into<String>, helper_path: impl Into<String>) -> Self {
        Self {
            runtime: Wsl2Runtime::new(distro, helper_path),
        }
    }
}

impl FilesystemProvider for Wsl2Filesystem {
    /// Set up the container rootfs overlay inside WSL2 and return the merged path.
    ///
    /// Translates all layer paths and the container directory to WSL2 paths,
    /// serialises the request to JSON, and calls `minibox-wsl-helper setup-rootfs`.
    /// The returned path is the WSL2-side merged directory path (not translated
    /// back to a Windows path).
    ///
    /// # Errors
    ///
    /// Returns an error if path translation fails, if the helper command fails,
    /// or if the JSON response cannot be parsed.
    fn setup_rootfs(&self, image_layers: &[PathBuf], container_dir: &Path) -> Result<PathBuf> {
        debug!(
            layers = ?image_layers,
            container_dir = ?container_dir,
            "wsl2: setting up rootfs"
        );

        let layers_wsl: Result<Vec<String>> = image_layers
            .iter()
            .map(|p| self.runtime.windows_to_wsl_path(p))
            .collect();

        let container_dir_wsl = self.runtime.windows_to_wsl_path(container_dir)?;

        let request = Wsl2FilesystemSetupRequest {
            layers: layers_wsl?,
            container_dir: container_dir_wsl,
        };

        let json = serde_json::to_string(&request)?;

        let output =
            self.runtime
                .wsl_exec(&["sudo", &self.runtime.helper_path, "setup-rootfs", &json])?;

        let response: Wsl2FilesystemSetupResponse = serde_json::from_str(&output)?;

        Ok(Path::new(&response.merged_path).to_path_buf())
    }

    /// No-op: `pivot_root` is performed by the helper inside the container process.
    ///
    /// When the container process is already running inside WSL2, the Linux
    /// helper handles `pivot_root(2)` directly as part of the spawn sequence.
    /// This adapter layer has nothing to do.
    fn pivot_root(&self, _new_root: &Path) -> Result<()> {
        debug!("wsl2: pivot_root delegated to helper (called inside container)");
        Ok(())
    }

    /// Tear down the container overlay and remove the container directory in WSL2.
    ///
    /// Calls `minibox-wsl-helper cleanup <container_dir_wsl>` inside the WSL2
    /// distribution.
    ///
    /// # Errors
    ///
    /// Returns an error if path translation or the helper command fails.
    fn cleanup(&self, container_dir: &Path) -> Result<()> {
        debug!(container_dir = ?container_dir, "wsl2: cleaning up filesystem");

        let container_dir_wsl = self.runtime.windows_to_wsl_path(container_dir)?;

        self.runtime.wsl_exec(&[
            "sudo",
            &self.runtime.helper_path,
            "cleanup",
            &container_dir_wsl,
        ])?;

        Ok(())
    }
}

/// WSL2-based implementation of [`ResourceLimiter`].
///
/// Delegates cgroups v2 operations to `minibox-wsl-helper` running inside
/// the named WSL2 distribution.
#[derive(Debug, Clone)]
pub struct Wsl2Limiter {
    /// Shared runtime handle used for path translation and WSL exec.
    runtime: Wsl2Runtime,
}

impl Wsl2Limiter {
    /// Create a new WSL2 resource limiter adapter.
    ///
    /// # Arguments
    ///
    /// * `distro` - WSL2 distribution name.
    /// * `helper_path` - Path to `minibox-wsl-helper` inside the WSL2 filesystem.
    pub fn new(distro: impl Into<String>, helper_path: impl Into<String>) -> Self {
        Self {
            runtime: Wsl2Runtime::new(distro, helper_path),
        }
    }
}

impl ResourceLimiter for Wsl2Limiter {
    /// Create a cgroup for `container_id` inside WSL2 and apply resource limits.
    ///
    /// Serialises the container ID and [`ResourceConfig`] to JSON and calls
    /// `minibox-wsl-helper create-cgroup`. Returns the WSL2-side cgroup path
    /// string from the helper's JSON response.
    ///
    /// # Errors
    ///
    /// Returns an error if the helper command fails or the JSON response cannot
    /// be parsed.
    fn create(&self, container_id: &str, config: &ResourceConfig) -> Result<String> {
        debug!(
            container_id = container_id,
            config = ?config,
            "wsl2: creating cgroup"
        );

        let request = WslCgroupCreateRequest {
            container_id: container_id.to_string(),
            config: config.clone(),
        };

        let json = serde_json::to_string(&request)?;

        let output =
            self.runtime
                .wsl_exec(&["sudo", &self.runtime.helper_path, "create-cgroup", &json])?;

        let response: WslCgroupCreateResponse = serde_json::from_str(&output)?;
        Ok(response.cgroup_path)
    }

    /// Add `pid` to the cgroup associated with `container_id` inside WSL2.
    ///
    /// Calls `minibox-wsl-helper add-process <container_id> <pid>` inside the
    /// WSL2 distribution.
    ///
    /// # Errors
    ///
    /// Returns an error if the helper command fails.
    fn add_process(&self, container_id: &str, pid: u32) -> Result<()> {
        debug!(
            container_id = container_id,
            pid = pid,
            "wsl2: adding process to cgroup"
        );

        self.runtime.wsl_exec(&[
            "sudo",
            &self.runtime.helper_path,
            "add-process",
            container_id,
            &pid.to_string(),
        ])?;

        Ok(())
    }

    /// Remove the cgroup for `container_id` inside WSL2.
    ///
    /// Calls `minibox-wsl-helper cleanup-cgroup <container_id>` inside the
    /// WSL2 distribution.
    ///
    /// # Errors
    ///
    /// Returns an error if the helper command fails.
    fn cleanup(&self, container_id: &str) -> Result<()> {
        debug!(container_id = container_id, "wsl2: cleaning up cgroup");

        self.runtime.wsl_exec(&[
            "sudo",
            &self.runtime.helper_path,
            "cleanup-cgroup",
            container_id,
        ])?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// WSL Helper Protocol Types
// ---------------------------------------------------------------------------

/// JSON request sent to `minibox-wsl-helper spawn`.
#[derive(Debug, Serialize, Deserialize)]
struct WslSpawnRequest {
    /// Absolute rootfs path inside the WSL2 distribution.
    rootfs: String,
    /// Container entrypoint binary path.
    command: String,
    /// Arguments passed to `command`.
    args: Vec<String>,
    /// Environment variables in `KEY=VALUE` form.
    env: Vec<String>,
    /// UTS hostname for the container.
    hostname: String,
    /// Cgroup path inside the WSL2 distribution.
    cgroup_path: String,
}

/// JSON response from `minibox-wsl-helper spawn`.
#[derive(Debug, Serialize, Deserialize)]
struct WslSpawnResponse {
    /// Linux PID of the spawned container process inside WSL2.
    pid: u32,
}

/// JSON request sent to `minibox-wsl-helper setup-rootfs`.
#[derive(Debug, Serialize, Deserialize)]
struct Wsl2FilesystemSetupRequest {
    /// Ordered list of layer paths (WSL2-side) used as overlay `lowerdir`.
    layers: Vec<String>,
    /// Container working directory (WSL2-side) for upper/work/merged subdirs.
    container_dir: String,
}

/// JSON response from `minibox-wsl-helper setup-rootfs`.
#[derive(Debug, Serialize, Deserialize)]
struct Wsl2FilesystemSetupResponse {
    /// Absolute path to the overlay merged directory inside WSL2.
    merged_path: String,
}

/// JSON request sent to `minibox-wsl-helper create-cgroup`.
#[derive(Debug, Serialize, Deserialize)]
struct WslCgroupCreateRequest {
    /// Container identifier used as the cgroup subdirectory name.
    container_id: String,
    /// Resource limits to apply to the new cgroup.
    config: ResourceConfig,
}

/// JSON response from `minibox-wsl-helper create-cgroup`.
#[derive(Debug, Serialize, Deserialize)]
struct WslCgroupCreateResponse {
    /// Absolute cgroup path inside WSL2 (e.g. `/sys/fs/cgroup/minibox/<id>`).
    cgroup_path: String,
}

// Register WSL2 adapters with the as_any! macro so they satisfy the AsAny
// bound required by the daemon's adapter registry. Note: there is no
// Wsl2Registry — image pulling uses DockerHubRegistry on the Windows side.
as_any!(Wsl2Runtime, Wsl2Filesystem, Wsl2Limiter);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wsl_runtime_creation() {
        let runtime = Wsl2Runtime::new("Ubuntu", "/usr/local/bin/minibox-wsl-helper");
        assert_eq!(runtime.distro, "Ubuntu");
        assert_eq!(runtime.helper_path, "/usr/local/bin/minibox-wsl-helper");
    }

    #[test]
    #[cfg(target_os = "windows")]
    #[ignore] // Requires WSL2 installed
    fn test_wsl_path_conversion() {
        let runtime = Wsl2Runtime::new("Ubuntu", "/usr/local/bin/minibox-wsl-helper");
        let windows_path = Path::new("C:\\Users\\test\\file.txt");

        let wsl_path = runtime.windows_to_wsl_path(windows_path);
        // Should convert to /mnt/c/Users/test/file.txt
        assert!(wsl_path.is_ok());
        assert!(wsl_path.unwrap().contains("/mnt/c/"));
    }
}
