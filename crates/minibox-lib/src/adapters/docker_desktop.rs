//! Docker Desktop adapter for macOS cross-platform support.
//!
//! This adapter delegates container operations to Docker Desktop's Linux VM,
//! enabling macOS users to run minibox containers.
//!
//! # Architecture
//!
//! ```text
//! macOS Host
//! ┌────────────────────────────────────────┐
//! │  miniboxd (macOS)                      │
//! │  ┌──────────────┐                      │
//! │  │ DockerDesktop│                      │
//! │  │ Runtime      │  ---docker--->       │
//! │  │ Filesystem   │                      │
//! │  │ Limiter      │                      │
//! │  └──────────────┘                      │
//! └────────────────────────────────────────┘
//!                 │
//!                 ▼
//! Docker Desktop VM (Linux)
//! ┌────────────────────────────────────────┐
//! │  minibox-docker-helper (container)     │
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
//! - macOS 10.15+ (Catalina or later)
//! - Docker Desktop 4.0+ installed and running
//! - `minibox-docker-helper` container image built
//!
//! # Setup
//!
//! ```bash
//! # Ensure Docker Desktop is running
//! docker version
//!
//! # Build helper container
//! docker build -t minibox-docker-helper:latest -f Dockerfile.helper .
//!
//! # Test helper
//! docker run --rm --privileged minibox-docker-helper:latest version
//! ```
//!
//! # Implementation Strategy
//!
//! Uses Docker Desktop's `docker` CLI to:
//! 1. Run privileged helper container with host PID/network
//! 2. Mount macOS volumes into helper container
//! 3. Execute minibox operations inside Linux environment
//! 4. Return results to macOS host

use crate::domain::{
    AsAny, ContainerRuntime, ContainerSpawnConfig, FilesystemProvider, ResourceConfig,
    ResourceLimiter, RuntimeCapabilities,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::debug;

/// Docker Desktop-based container runtime for macOS.
///
/// Delegates all container operations to a privileged helper container
/// running in Docker Desktop's Linux VM.
#[derive(Debug, Clone)]
pub struct DockerDesktopRuntime {
    /// Docker helper container image
    helper_image: String,
    /// Optional custom docker executable path
    docker_bin: String,
}

impl DockerDesktopRuntime {
    /// Create a new Docker Desktop runtime adapter.
    ///
    /// # Arguments
    ///
    /// * `helper_image` - Docker image for helper (e.g., "minibox-docker-helper:latest")
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let runtime = DockerDesktopRuntime::new("minibox-docker-helper:latest");
    /// ```
    pub fn new(helper_image: impl Into<String>) -> Self {
        Self {
            helper_image: helper_image.into(),
            docker_bin: "docker".to_string(),
        }
    }

    /// Use a custom docker binary path.
    pub fn with_docker_bin(mut self, path: impl Into<String>) -> Self {
        self.docker_bin = path.into();
        self
    }

    /// Execute a command in the helper container.
    fn docker_exec(&self, args: &[&str], json_input: Option<&str>) -> Result<String> {
        let mut cmd = Command::new(&self.docker_bin);

        // Run helper container with required privileges
        cmd.arg("run")
            .arg("--rm") // Remove container after execution
            .arg("--privileged") // Required for namespaces
            .arg("--pid=host") // Access host PID namespace (Docker VM)
            .arg("--network=host") // Access host network
            .arg(&self.helper_image);

        // Add command arguments
        cmd.args(args);

        // If JSON input provided, pass via stdin
        if let Some(json) = json_input {
            cmd.arg(json);
        }

        let output = cmd.output().context("failed to execute docker command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Docker helper command failed: {}", stderr);
        }

        String::from_utf8(output.stdout).context("invalid UTF-8 from helper")
    }

    /// Convert macOS path to Docker-mounted path.
    ///
    /// Docker Desktop automatically mounts macOS paths:
    /// - /Users -> /Users (macOS paths accessible directly)
    /// - /Volumes -> /Volumes
    /// - /private -> /private
    fn macos_to_docker_path(&self, macos_path: &Path) -> Result<String> {
        // Docker Desktop maps macOS paths directly
        macos_path
            .to_str()
            .context("invalid path")
            .map(|s| s.to_string())
    }
}

impl AsAny for DockerDesktopRuntime {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[async_trait]
impl ContainerRuntime for DockerDesktopRuntime {
    fn capabilities(&self) -> RuntimeCapabilities {
        // Docker Desktop runs a Linux VM — cgroups/overlay/network available,
        // but user namespace remapping is managed by Docker itself
        RuntimeCapabilities {
            supports_user_namespaces: false,
            supports_cgroups_v2: true,
            supports_overlay_fs: true,
            supports_network_isolation: true,
            max_containers: None,
        }
    }

    async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<u32> {
        debug!(
            "spawning container via Docker Desktop: command={}, rootfs={:?}",
            config.command, config.rootfs
        );

        // Convert macOS paths to Docker-accessible paths
        let rootfs_docker = self.macos_to_docker_path(&config.rootfs)?;
        let cgroup_path_docker = self.macos_to_docker_path(&config.cgroup_path)?;

        // Serialize spawn config
        let spawn_request = DockerSpawnRequest {
            rootfs: rootfs_docker,
            command: config.command.clone(),
            args: config.args.clone(),
            env: config.env.clone(),
            hostname: config.hostname.clone(),
            cgroup_path: cgroup_path_docker,
        };

        let json = serde_json::to_string(&spawn_request)?;

        // Call Docker helper in blocking thread
        let helper_image = self.helper_image.clone();
        let docker_bin = self.docker_bin.clone();

        let output = tokio::task::spawn_blocking(move || {
            let mut cmd = Command::new(&docker_bin);
            cmd.arg("run")
                .arg("--rm")
                .arg("--privileged")
                .arg("--pid=host")
                .arg(&helper_image)
                .arg("spawn")
                .arg(&json);

            cmd.output()
        })
        .await?
        .context("failed to execute Docker helper")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Docker helper spawn failed: {}", stderr);
        }

        // Parse PID from stdout
        let stdout = String::from_utf8(output.stdout)?;
        let response: DockerSpawnResponse = serde_json::from_str(&stdout)?;

        debug!("container spawned in Docker Desktop with PID {}", response.pid);
        Ok(response.pid)
    }
}

/// Docker Desktop-based filesystem provider.
#[derive(Debug, Clone)]
pub struct DockerDesktopFilesystem {
    runtime: DockerDesktopRuntime,
}

impl DockerDesktopFilesystem {
    pub fn new(helper_image: impl Into<String>) -> Self {
        Self {
            runtime: DockerDesktopRuntime::new(helper_image),
        }
    }
}

impl AsAny for DockerDesktopFilesystem {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl FilesystemProvider for DockerDesktopFilesystem {
    fn setup_rootfs(&self, image_layers: &[PathBuf], container_dir: &Path) -> Result<PathBuf> {
        debug!(
            "setting up rootfs via Docker Desktop: layers={:?}, dir={:?}",
            image_layers, container_dir
        );

        // Convert paths to Docker-accessible
        let layers_docker: Result<Vec<String>> = image_layers
            .iter()
            .map(|p| self.runtime.macos_to_docker_path(p))
            .collect();

        let container_dir_docker = self.runtime.macos_to_docker_path(container_dir)?;

        let request = DockerFilesystemSetupRequest {
            layers: layers_docker?,
            container_dir: container_dir_docker,
        };

        let json = serde_json::to_string(&request)?;

        let output = self.runtime.docker_exec(&["setup-rootfs"], Some(&json))?;

        let response: DockerFilesystemSetupResponse = serde_json::from_str(&output)?;

        // Docker paths map directly back to macOS
        Ok(PathBuf::from(response.merged_path))
    }

    fn pivot_root(&self, _new_root: &Path) -> Result<()> {
        // Called inside container process, handled by helper
        debug!("pivot_root delegated to Docker helper");
        Ok(())
    }

    fn cleanup(&self, container_dir: &Path) -> Result<()> {
        debug!("cleaning up filesystem via Docker Desktop: dir={:?}", container_dir);

        let container_dir_docker = self.runtime.macos_to_docker_path(container_dir)?;

        self.runtime.docker_exec(&["cleanup", &container_dir_docker], None)?;

        Ok(())
    }
}

/// Docker Desktop-based resource limiter.
#[derive(Debug, Clone)]
pub struct DockerDesktopLimiter {
    runtime: DockerDesktopRuntime,
}

impl DockerDesktopLimiter {
    pub fn new(helper_image: impl Into<String>) -> Self {
        Self {
            runtime: DockerDesktopRuntime::new(helper_image),
        }
    }
}

impl AsAny for DockerDesktopLimiter {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ResourceLimiter for DockerDesktopLimiter {
    fn create(&self, container_id: &str, config: &ResourceConfig) -> Result<String> {
        debug!(
            "creating cgroup via Docker Desktop: id={}, config={:?}",
            container_id, config
        );

        let request = DockerCgroupCreateRequest {
            container_id: container_id.to_string(),
            config: config.clone(),
        };

        let json = serde_json::to_string(&request)?;

        let output = self.runtime.docker_exec(&["create-cgroup"], Some(&json))?;

        let response: DockerCgroupCreateResponse = serde_json::from_str(&output)?;
        Ok(response.cgroup_path)
    }

    fn add_process(&self, container_id: &str, pid: u32) -> Result<()> {
        debug!(
            "adding process {} to cgroup {} via Docker Desktop",
            pid, container_id
        );

        self.runtime.docker_exec(&["add-process", container_id, &pid.to_string()], None)?;

        Ok(())
    }

    fn cleanup(&self, container_id: &str) -> Result<()> {
        debug!("cleaning up cgroup {} via Docker Desktop", container_id);

        self.runtime.docker_exec(&["cleanup-cgroup", container_id], None)?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Docker Helper Protocol Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct DockerSpawnRequest {
    rootfs: String,
    command: String,
    args: Vec<String>,
    env: Vec<String>,
    hostname: String,
    cgroup_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DockerSpawnResponse {
    pid: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct DockerFilesystemSetupRequest {
    layers: Vec<String>,
    container_dir: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DockerFilesystemSetupResponse {
    merged_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DockerCgroupCreateRequest {
    container_id: String,
    config: ResourceConfig,
}

#[derive(Debug, Serialize, Deserialize)]
struct DockerCgroupCreateResponse {
    cgroup_path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_docker_desktop_runtime_creation() {
        let runtime = DockerDesktopRuntime::new("minibox-docker-helper:latest");
        assert_eq!(runtime.helper_image, "minibox-docker-helper:latest");
        assert_eq!(runtime.docker_bin, "docker");
    }

    #[test]
    fn test_custom_docker_bin() {
        let runtime = DockerDesktopRuntime::new("helper:latest")
            .with_docker_bin("/usr/local/bin/docker");
        assert_eq!(runtime.docker_bin, "/usr/local/bin/docker");
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[ignore] // Requires Docker Desktop running
    fn test_macos_path_mapping() {
        let runtime = DockerDesktopRuntime::new("helper:latest");
        let macos_path = Path::new("/Users/test/file.txt");

        let docker_path = runtime.macos_to_docker_path(macos_path);
        assert!(docker_path.is_ok());
        // Docker Desktop maps /Users directly
        assert_eq!(docker_path.unwrap(), "/Users/test/file.txt");
    }
}
