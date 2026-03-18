//! Windows Subsystem for Linux (WSL) adapter for cross-platform support.
//!
//! This adapter delegates container operations to a Linux environment running
//! in WSL2, enabling Windows users to run minibox containers.
//!
//! # Architecture
//!
//! ```text
//! Windows Host
//! ┌────────────────────────────────────────┐
//! │  miniboxd (Windows)                    │
//! │  ┌──────────────┐                      │
//! │  │ WslRuntime   │                      │
//! │  │ WslFilesystem│  ---wsl.exe--->      │
//! │  │ WslLimiter   │                      │
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
//! - WSL2 distribution (Ubuntu 20.04+ recommended)
//! - `minibox-wsl-helper` binary installed in WSL
//!
//! # Setup
//!
//! ```bash
//! # Install WSL2
//! wsl --install
//!
//! # Build helper in WSL
//! wsl -d Ubuntu cargo build --release -p minibox-wsl-helper
//!
//! # Copy to WSL bin
//! wsl -d Ubuntu sudo cp target/release/minibox-wsl-helper /usr/local/bin/
//! ```

use crate::{
    as_any,
    domain::{
        ContainerRuntime, ContainerSpawnConfig, FilesystemProvider, ResourceConfig,
        ResourceLimiter, RuntimeCapabilities,
    },
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::debug;

/// WSL2-based container runtime for Windows.
///
/// Delegates all container operations to a Linux helper binary running in WSL2.
#[derive(Debug, Clone)]
pub struct WslRuntime {
    /// WSL distribution name (e.g., "Ubuntu", "Debian")
    distro: String,
    /// Path to helper binary in WSL (e.g., "/usr/local/bin/minibox-wsl-helper")
    helper_path: String,
}

impl WslRuntime {
    /// Create a new WSL runtime adapter.
    ///
    /// # Arguments
    ///
    /// * `distro` - WSL distribution name (run `wsl -l` to see available)
    /// * `helper_path` - Path to minibox-wsl-helper in WSL filesystem
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let runtime = WslRuntime::new("Ubuntu", "/usr/local/bin/minibox-wsl-helper");
    /// ```
    pub fn new(distro: impl Into<String>, helper_path: impl Into<String>) -> Self {
        Self {
            distro: distro.into(),
            helper_path: helper_path.into(),
        }
    }

    /// Execute a command in WSL and capture output.
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

    /// Convert Windows path to WSL path (e.g., C:\Users\... -> /mnt/c/Users/...)
    fn windows_to_wsl_path(&self, windows_path: &Path) -> Result<String> {
        let path_str = windows_path.to_str().context("invalid path")?;

        // Use wsl.exe to convert path
        let wsl_path = self
            .wsl_exec(&["wslpath", "-u", path_str])
            .context("failed to convert Windows path to WSL path")?;

        Ok(wsl_path.trim().to_string())
    }
}

#[async_trait]
impl ContainerRuntime for WslRuntime {
    fn capabilities(&self) -> RuntimeCapabilities {
        // WSL2 delegates to a full Linux kernel — all features available
        RuntimeCapabilities {
            supports_user_namespaces: true,
            supports_cgroups_v2: true,
            supports_overlay_fs: true,
            supports_network_isolation: true,
            max_containers: None,
        }
    }

    async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<u32> {
        debug!(
            "spawning container via WSL: command={}, rootfs={:?}",
            config.command, config.rootfs
        );

        // Convert Windows paths to WSL paths
        let rootfs_wsl = self.windows_to_wsl_path(&config.rootfs)?;
        let cgroup_path_wsl = self.windows_to_wsl_path(&config.cgroup_path)?;

        // Serialize spawn config to JSON
        let spawn_request = WslSpawnRequest {
            rootfs: rootfs_wsl,
            command: config.command.clone(),
            args: config.args.clone(),
            env: config.env.clone(),
            hostname: config.hostname.clone(),
            cgroup_path: cgroup_path_wsl,
        };

        let json = serde_json::to_string(&spawn_request).context("failed to serialize request")?;

        // Call WSL helper in blocking thread
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

        // Parse PID from stdout
        let stdout = String::from_utf8(output.stdout).context("invalid UTF-8 from helper")?;
        let response: WslSpawnResponse =
            serde_json::from_str(&stdout).context("failed to parse spawn response")?;

        debug!("container spawned in WSL with PID {}", response.pid);
        Ok(response.pid)
    }
}

/// WSL2-based filesystem provider.
///
/// Delegates overlay filesystem operations to WSL2.
#[derive(Debug, Clone)]
pub struct WslFilesystem {
    runtime: WslRuntime,
}

impl WslFilesystem {
    pub fn new(distro: impl Into<String>, helper_path: impl Into<String>) -> Self {
        Self {
            runtime: WslRuntime::new(distro, helper_path),
        }
    }
}

impl FilesystemProvider for WslFilesystem {
    fn setup_rootfs(&self, image_layers: &[PathBuf], container_dir: &Path) -> Result<PathBuf> {
        debug!(
            "setting up rootfs via WSL: layers={:?}, dir={:?}",
            image_layers, container_dir
        );

        // Convert paths to WSL
        let layers_wsl: Result<Vec<String>> = image_layers
            .iter()
            .map(|p| self.runtime.windows_to_wsl_path(p))
            .collect();

        let container_dir_wsl = self.runtime.windows_to_wsl_path(container_dir)?;

        let request = WslFilesystemSetupRequest {
            layers: layers_wsl?,
            container_dir: container_dir_wsl,
        };

        let json = serde_json::to_string(&request)?;

        // Execute setup command
        let output =
            self.runtime
                .wsl_exec(&["sudo", &self.runtime.helper_path, "setup-rootfs", &json])?;

        let response: WslFilesystemSetupResponse = serde_json::from_str(&output)?;

        // Convert WSL path back to Windows
        let merged_wsl_path = Path::new(&response.merged_path);
        Ok(merged_wsl_path.to_path_buf())
    }

    fn pivot_root(&self, _new_root: &Path) -> Result<()> {
        // This is called inside the container process, which is already in WSL
        // The Linux helper handles pivot_root directly
        debug!("pivot_root delegated to WSL helper (called inside container)");
        Ok(())
    }

    fn cleanup(&self, container_dir: &Path) -> Result<()> {
        debug!("cleaning up filesystem via WSL: dir={:?}", container_dir);

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

/// WSL2-based resource limiter.
///
/// Delegates cgroups operations to WSL2.
#[derive(Debug, Clone)]
pub struct WslLimiter {
    runtime: WslRuntime,
}

impl WslLimiter {
    pub fn new(distro: impl Into<String>, helper_path: impl Into<String>) -> Self {
        Self {
            runtime: WslRuntime::new(distro, helper_path),
        }
    }
}

impl ResourceLimiter for WslLimiter {
    fn create(&self, container_id: &str, config: &ResourceConfig) -> Result<String> {
        debug!(
            "creating cgroup via WSL: id={}, config={:?}",
            container_id, config
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

    fn add_process(&self, container_id: &str, pid: u32) -> Result<()> {
        debug!("adding process {} to cgroup {} via WSL", pid, container_id);

        self.runtime.wsl_exec(&[
            "sudo",
            &self.runtime.helper_path,
            "add-process",
            container_id,
            &pid.to_string(),
        ])?;

        Ok(())
    }

    fn cleanup(&self, container_id: &str) -> Result<()> {
        debug!("cleaning up cgroup {} via WSL", container_id);

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

/// Request to spawn a container process in WSL.
#[derive(Debug, Serialize, Deserialize)]
struct WslSpawnRequest {
    rootfs: String,
    command: String,
    args: Vec<String>,
    env: Vec<String>,
    hostname: String,
    cgroup_path: String,
}

/// Response from spawning a container process.
#[derive(Debug, Serialize, Deserialize)]
struct WslSpawnResponse {
    pid: u32,
}

/// Request to setup overlay filesystem.
#[derive(Debug, Serialize, Deserialize)]
struct WslFilesystemSetupRequest {
    layers: Vec<String>,
    container_dir: String,
}

/// Response from filesystem setup.
#[derive(Debug, Serialize, Deserialize)]
struct WslFilesystemSetupResponse {
    merged_path: String,
}

/// Request to create cgroup.
#[derive(Debug, Serialize, Deserialize)]
struct WslCgroupCreateRequest {
    container_id: String,
    config: ResourceConfig,
}

/// Response from cgroup creation.
#[derive(Debug, Serialize, Deserialize)]
struct WslCgroupCreateResponse {
    cgroup_path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wsl_runtime_creation() {
        let runtime = WslRuntime::new("Ubuntu", "/usr/local/bin/minibox-wsl-helper");
        assert_eq!(runtime.distro, "Ubuntu");
        assert_eq!(runtime.helper_path, "/usr/local/bin/minibox-wsl-helper");
    }

    #[test]
    #[cfg(target_os = "windows")]
    #[ignore] // Requires WSL2 installed
    fn test_wsl_path_conversion() {
        let runtime = WslRuntime::new("Ubuntu", "/usr/local/bin/minibox-wsl-helper");
        let windows_path = Path::new("C:\\Users\\test\\file.txt");

        let wsl_path = runtime.windows_to_wsl_path(windows_path);
        // Should convert to /mnt/c/Users/test/file.txt
        assert!(wsl_path.is_ok());
        assert!(wsl_path.unwrap().contains("/mnt/c/"));
    }
}

as_any!(WslRuntime, WslFilesystem, WslLimiter);
