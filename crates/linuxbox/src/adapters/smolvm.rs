//! SmolVM adapter suite for macOS support via lightweight Linux VMs.
//!
//! This module provides four adapters that together implement the full set of
//! domain traits ([`ImageRegistry`], [`FilesystemProvider`], [`ResourceLimiter`],
//! [`ContainerRuntime`]) by delegating operations into a smolvm Linux VM
//! running on the macOS host.
//!
//! # How it works
//!
//! Each adapter runs commands inside a smolvm VM using
//! `smolvm machine run --image <image> -- <command>`. smolvm boots a Linux VM
//! in under 1 second using Apple's Virtualization.framework and caches images
//! locally after first pull.
//!
//! # Adapter selection
//!
//! Selected by `MINIBOX_ADAPTER=smolvm`. These adapters are compiled on all
//! platforms but are only useful on macOS where smolvm is available.
//!
//! # Requirements
//!
//! - smolvm installed (`brew install smolvm`)
//! - macOS with Apple Silicon or Intel

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use minibox_core::adapt;
use minibox_core::domain::{
    ContainerRuntime, ContainerSpawnConfig, ImageMetadata, ImageRegistry, ResourceConfig,
    ResourceLimiter, RootfsLayout, RuntimeCapabilities, SpawnResult,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

/// Default smolvm image used for container operations.
const DEFAULT_IMAGE: &str = "ubuntu:24.04";

/// Callable that runs a command inside the smolvm VM and returns its stdout.
///
/// The default implementation invokes `smolvm machine run --image <image> -- <args...>`.
/// Tests inject a fake closure via the `with_executor` builder methods to avoid
/// real smolvm calls.
pub type SmolVmExecutor = Arc<dyn Fn(&[&str]) -> Result<String> + Send + Sync>;

/// Run a command via the real `smolvm` binary and return stdout.
fn smolvm_exec(image: &str, args: &[&str]) -> Result<String> {
    let output = Command::new("smolvm")
        .args([
            "machine",
            "run",
            "--net",
            "--image",
            image,
            "--timeout",
            "60s",
            "--",
        ])
        .args(args)
        .output()
        .map_err(|e| anyhow!("failed to execute smolvm: {e}"))?;

    if !output.status.success() {
        return Err(anyhow!(
            "smolvm command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Run a command via the real `smolvm` binary with volume mounts and env vars.
fn smolvm_exec_full(
    image: &str,
    args: &[&str],
    volumes: &[(&str, &str)],
    env: &[(&str, &str)],
    timeout_secs: u32,
) -> Result<String> {
    let mut cmd = Command::new("smolvm");
    cmd.args(["machine", "run", "--net", "--image", image]);
    cmd.args(["--timeout", &format!("{timeout_secs}s")]);

    for (host, guest) in volumes {
        cmd.args(["-v", &format!("{host}:{guest}")]);
    }
    for (key, val) in env {
        cmd.args(["-e", &format!("{key}={val}")]);
    }

    cmd.arg("--");
    cmd.args(args);

    let output = cmd
        .output()
        .map_err(|e| anyhow!("failed to execute smolvm: {e}"))?;

    if !output.status.success() {
        return Err(anyhow!(
            "smolvm command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// ============================================================================
// SmolVm Image Registry Adapter
// ============================================================================

/// SmolVM implementation of [`ImageRegistry`].
///
/// Pulls images via `smolvm machine run` which handles image management
/// internally. smolvm caches images locally after first pull.
pub struct SmolVmRegistry {
    /// Image to use for the VM (default: ubuntu:24.04).
    image: String,
    /// Optional injected executor used in tests to avoid real smolvm calls.
    executor: Option<SmolVmExecutor>,
}

impl SmolVmRegistry {
    /// Create a new registry adapter using the default smolvm image.
    pub fn new() -> Self {
        Self {
            image: DEFAULT_IMAGE.to_string(),
            executor: None,
        }
    }

    /// Override the smolvm VM image (default: `ubuntu:24.04`).
    pub fn with_image(mut self, image: String) -> Self {
        self.image = image;
        self
    }

    /// Inject a custom executor for testing.
    ///
    /// The closure receives the argument slice that would be passed to
    /// `smolvm machine run -- <args>` and must return the command's stdout.
    pub fn with_executor(mut self, executor: SmolVmExecutor) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Run a command inside the smolvm VM and return its stdout.
    fn vm_exec(&self, args: &[&str]) -> Result<String> {
        if let Some(exec) = &self.executor {
            return exec(args);
        }
        smolvm_exec(&self.image, args)
    }
}

#[async_trait]
impl ImageRegistry for SmolVmRegistry {
    /// Check if an image is available inside the smolvm VM.
    ///
    /// Runs `docker images --filter reference=<name>:<tag> --quiet` inside
    /// the VM. Returns `true` if the output is non-empty.
    async fn has_image(&self, name: &str, tag: &str) -> bool {
        let short_name = name.strip_prefix("library/").unwrap_or(name);
        let full_name = format!("{short_name}:{tag}");
        self.vm_exec(&[
            "docker",
            "images",
            "--filter",
            &format!("reference={full_name}"),
            "--quiet",
        ])
        .map(|out| !out.trim().is_empty())
        .unwrap_or(false)
    }

    /// Pull an image inside the smolvm VM via `docker pull`.
    async fn pull_image(
        &self,
        image_ref: &crate::image::reference::ImageRef,
    ) -> Result<ImageMetadata> {
        let cache_name = image_ref.cache_name();
        let tag = image_ref.tag.clone();
        let full_name = format!("{cache_name}:{tag}");

        self.vm_exec(&["docker", "pull", &full_name])?;

        Ok(ImageMetadata {
            name: cache_name,
            tag,
            layers: vec![],
        })
    }

    /// Layer paths live inside the VM's filesystem. Returning an empty vec
    /// signals to the caller that it should pull first.
    fn get_image_layers(&self, _name: &str, _tag: &str) -> Result<Vec<PathBuf>> {
        Ok(vec![])
    }
}

// ============================================================================
// SmolVm Container Runtime Adapter
// ============================================================================

/// SmolVM implementation of [`ContainerRuntime`].
///
/// Spawns container processes by running commands inside a smolvm VM via
/// `smolvm machine run`. Each `spawn_process` call boots a fresh VM instance.
pub struct SmolVmRuntime {
    /// Image to use for the VM.
    image: String,
    /// Optional injected executor used in tests.
    executor: Option<SmolVmExecutor>,
}

impl SmolVmRuntime {
    /// Create a new runtime adapter using the default smolvm image.
    pub fn new() -> Self {
        Self {
            image: DEFAULT_IMAGE.to_string(),
            executor: None,
        }
    }

    /// Inject a custom executor for testing.
    pub fn with_executor(mut self, executor: SmolVmExecutor) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Run a command inside the smolvm VM and return its stdout.
    fn vm_exec(&self, args: &[&str]) -> Result<String> {
        if let Some(exec) = &self.executor {
            return exec(args);
        }
        smolvm_exec(&self.image, args)
    }
}

#[async_trait]
impl ContainerRuntime for SmolVmRuntime {
    /// smolvm capabilities: the VM provides a full Linux kernel with cgroups,
    /// overlay FS, and network isolation. User namespaces depend on the VM
    /// kernel config.
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            supports_user_namespaces: false,
            supports_cgroups_v2: true,
            supports_overlay_fs: true,
            supports_network_isolation: true,
            max_containers: None,
        }
    }

    /// Spawn a process inside a smolvm VM.
    ///
    /// Builds the command from `config.command` + `config.args`, passes
    /// environment variables and bind mounts, and runs via smolvm.
    async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<SpawnResult> {
        let mut command = vec![config.command.as_str()];
        let args: Vec<&str> = config.args.iter().map(|s| s.as_str()).collect();
        command.extend(&args);

        // Build volume and env args for smolvm.
        let volumes: Vec<(String, String)> = config
            .mounts
            .iter()
            .map(|m| {
                (
                    m.host_path.to_string_lossy().to_string(),
                    m.container_path.to_string_lossy().to_string(),
                )
            })
            .collect();
        let env_pairs: Vec<(String, String)> = config
            .env
            .iter()
            .filter_map(|entry| {
                entry
                    .split_once('=')
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
            })
            .collect();

        let vol_refs: Vec<(&str, &str)> = volumes
            .iter()
            .map(|(h, g)| (h.as_str(), g.as_str()))
            .collect();
        let env_refs: Vec<(&str, &str)> = env_pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        if self.executor.is_some() {
            // Use the test executor — flatten command into a single arg list.
            self.vm_exec(&command)?;
        } else {
            smolvm_exec_full(&self.image, &command, &vol_refs, &env_refs, 60)?;
        }

        Ok(SpawnResult {
            pid: 0,
            output_reader: None,
        })
    }
}

// ============================================================================
// SmolVm Filesystem Adapter
// ============================================================================

/// SmolVM implementation of [`FilesystemProvider`].
///
/// Filesystem operations are handled inside the VM. All methods are no-ops
/// on the host side — the VM's kernel manages overlay mounts and pivot_root.
pub struct SmolVmFilesystem;

impl SmolVmFilesystem {
    /// Create a new filesystem adapter.
    pub fn new() -> Self {
        Self
    }
}

impl minibox_core::domain::RootfsSetup for SmolVmFilesystem {
    /// Delegated to the VM — return a placeholder layout.
    fn setup_rootfs(
        &self,
        _image_layers: &[PathBuf],
        container_dir: &Path,
    ) -> Result<RootfsLayout> {
        tracing::debug!(
            container_dir = %container_dir.display(),
            "smolvm: setup_rootfs delegated to in-VM kernel (no-op on host)"
        );
        Ok(RootfsLayout {
            merged_dir: container_dir.to_path_buf(),
            rootfs_metadata: None,
            source_image_ref: None,
        })
    }

    /// Cleanup is handled by the VM on exit.
    fn cleanup(&self, container_dir: &Path) -> Result<()> {
        tracing::debug!(
            container_dir = %container_dir.display(),
            "smolvm: filesystem cleanup delegated to VM (no-op on host)"
        );
        Ok(())
    }
}

impl minibox_core::domain::ChildInit for SmolVmFilesystem {
    /// pivot_root runs inside the VM, not on the host.
    fn pivot_root(&self, new_root: &Path) -> Result<()> {
        tracing::debug!(
            new_root = %new_root.display(),
            "smolvm: pivot_root delegated to VM (no-op on host)"
        );
        Ok(())
    }
}

// ============================================================================
// SmolVm Resource Limiter Adapter
// ============================================================================

/// SmolVM implementation of [`ResourceLimiter`].
///
/// Cgroup operations are handled inside the VM's Linux kernel. All methods
/// are no-ops on the macOS host side.
pub struct SmolVmLimiter;

impl SmolVmLimiter {
    /// Create a new resource limiter adapter.
    pub fn new() -> Self {
        Self
    }
}

impl ResourceLimiter for SmolVmLimiter {
    /// Cgroup creation is handled inside the VM.
    fn create(&self, container_id: &str, _config: &ResourceConfig) -> Result<String> {
        tracing::debug!(
            container_id,
            "smolvm: resource limiter create delegated to VM (no-op on host)"
        );
        Ok(container_id.to_owned())
    }

    /// PID is inside the VM's PID namespace.
    fn add_process(&self, container_id: &str, pid: u32) -> Result<()> {
        tracing::debug!(
            container_id,
            pid,
            "smolvm: add_process delegated to VM (no-op on host)"
        );
        Ok(())
    }

    /// Cgroup cleanup is handled by the VM.
    fn cleanup(&self, container_id: &str) -> Result<()> {
        tracing::debug!(
            container_id,
            "smolvm: resource limiter cleanup delegated to VM (no-op on host)"
        );
        Ok(())
    }
}

// Register all four SmolVM adapters with the adapt! macro.
adapt!(
    SmolVmRegistry,
    SmolVmFilesystem,
    SmolVmLimiter,
    SmolVmRuntime
);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::domain::{FilesystemProvider, RootfsSetup};

    fn _assert_image_registry<T: ImageRegistry>() {}
    fn _assert_container_runtime<T: ContainerRuntime>() {}
    fn _assert_filesystem_provider<T: FilesystemProvider>() {}
    fn _assert_resource_limiter<T: ResourceLimiter>() {}

    /// Compile-time check: all four adapters satisfy the required domain traits.
    #[test]
    fn adapter_implements_all_traits() {
        let _ = _assert_image_registry::<SmolVmRegistry>;
        let _ = _assert_container_runtime::<SmolVmRuntime>;
        let _ = _assert_filesystem_provider::<SmolVmFilesystem>;
        let _ = _assert_resource_limiter::<SmolVmLimiter>;
    }

    /// Registry with injected executor returns true when image exists.
    #[tokio::test]
    async fn registry_has_image_with_executor() {
        let registry = SmolVmRegistry::new()
            .with_executor(Arc::new(|_args: &[&str]| Ok("sha256:abc123\n".to_string())));
        assert!(registry.has_image("alpine", "latest").await);
    }

    /// Registry with injected executor returns false on empty output.
    #[tokio::test]
    async fn registry_has_image_returns_false_on_empty() {
        let registry =
            SmolVmRegistry::new().with_executor(Arc::new(|_args: &[&str]| Ok(String::new())));
        assert!(!registry.has_image("alpine", "latest").await);
    }

    /// Pull failure propagates through the executor.
    #[tokio::test]
    async fn registry_pull_failure_propagates() {
        let registry = SmolVmRegistry::new().with_executor(Arc::new(|args: &[&str]| {
            if args.contains(&"pull") {
                Err(anyhow!("network timeout"))
            } else {
                Ok(String::new())
            }
        }));

        let result = registry
            .pull_image(&crate::image::reference::ImageRef::parse("alpine:3.18").expect("parse"))
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("network"));
    }

    /// Filesystem setup_rootfs returns the container_dir as merged_dir.
    #[test]
    fn filesystem_setup_rootfs_returns_placeholder() {
        let fs = SmolVmFilesystem::new();
        let dir = PathBuf::from("/tmp/test-container");
        let layout = fs.setup_rootfs(&[], &dir).expect("setup_rootfs");
        assert_eq!(layout.merged_dir, dir);
    }

    /// Limiter create returns the container ID.
    #[test]
    fn limiter_create_returns_id() {
        let limiter = SmolVmLimiter::new();
        let id = limiter
            .create("test-123", &ResourceConfig::default())
            .expect("create");
        assert_eq!(id, "test-123");
    }

    /// Runtime capabilities report cgroups v2 and overlay FS support.
    #[test]
    fn runtime_capabilities() {
        let runtime = SmolVmRuntime::new();
        let caps = runtime.capabilities();
        assert!(caps.supports_cgroups_v2);
        assert!(caps.supports_overlay_fs);
        assert!(caps.supports_network_isolation);
        assert!(!caps.supports_user_namespaces);
    }
}
