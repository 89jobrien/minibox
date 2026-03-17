//! Colima adapter for macOS support via Lima VMs.
//!
//! This adapter delegates container operations to a Colima (Lima) VM, enabling
//! minibox to run on macOS by executing Linux-specific operations inside the VM.
//!
//! Architecture:
//! - Uses `limactl` command to interact with Lima VM
//! - SSH into Colima VM to execute container operations
//! - Path translation between macOS host and Lima VM
//! - Direct containerd/cgroup access inside VM (no helper container needed)
//!
//! Requirements:
//! - Colima installed (`brew install colima`)
//! - Colima VM running (`colima start`)

use crate::{adapt};
use crate::domain::{
    ContainerRuntime, ContainerSpawnConfig, FilesystemProvider, ImageMetadata,
    ImageRegistry, ResourceConfig, ResourceLimiter, RuntimeCapabilities,
};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

/// Injectable executor for Lima VM commands.
///
/// Accepts a slice of arguments (as passed to `limactl shell <instance>`)
/// and returns stdout as a String.  The default implementation invokes a
/// real `limactl` subprocess; tests inject a fake closure instead.
type LimaExecutor = Arc<dyn Fn(&[&str]) -> Result<String> + Send + Sync>;

// ============================================================================
// Colima Image Registry Adapter
// ============================================================================

/// Colima adapter for ImageRegistry trait.
///
/// Delegates image pulling to containerd inside the Colima VM using `nerdctl pull`.
pub struct ColimaRegistry {
    /// Lima instance name (usually "colima")
    instance: String,
    /// Path to limactl binary
    limactl_path: String,
    /// Optional injected executor (used in tests to avoid real limactl calls).
    executor: Option<LimaExecutor>,
}

impl ColimaRegistry {
    pub fn new() -> Self {
        Self {
            instance: "colima".to_string(),
            limactl_path: "limactl".to_string(),
            executor: None,
        }
    }

    pub fn with_instance(mut self, instance: String) -> Self {
        self.instance = instance;
        self
    }

    pub fn with_executor(mut self, executor: LimaExecutor) -> Self {
        self.executor = Some(executor);
        self
    }

    fn lima_exec(&self, args: &[&str]) -> Result<String> {
        if let Some(exec) = &self.executor {
            return exec(args);
        }
        let output = Command::new(&self.limactl_path)
            .arg("shell")
            .arg(&self.instance)
            .args(args)
            .output()
            .map_err(|e| anyhow!("Failed to execute limactl: {e}"))?;

        if !output.status.success() {
            return Err(anyhow!(
                "Lima command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Convert macOS path to Lima VM path
    #[allow(dead_code)]
    fn macos_to_lima_path(&self, macos_path: &Path) -> Result<String> {
        // Colima mounts host filesystem at /Users, /tmp, etc.
        let path_str = macos_path
            .to_str()
            .ok_or_else(|| anyhow!("Invalid path encoding".to_string()))?;

        // Lima VM typically mirrors common macOS paths
        if path_str.starts_with("/Users/") || path_str.starts_with("/tmp/") {
            Ok(path_str.to_string())
        } else {
            // For other paths, they might not be mounted
            Err(anyhow!(
                "Path not mounted in Lima VM: {path_str}. Only /Users and /tmp are typically mounted."
            ))
        }
    }
}

#[async_trait]
impl ImageRegistry for ColimaRegistry {
    async fn has_image(&self, name: &str, tag: &str) -> bool {
        let full_name = format!("{name}:{tag}");
        let result = self.lima_exec(&["nerdctl", "image", "inspect", &full_name]);
        result.is_ok()
    }

    async fn pull_image(&self, name: &str, tag: &str) -> Result<ImageMetadata> {
        let full_name = format!("{name}:{tag}");

        // Pull image using nerdctl inside Colima VM
        self.lima_exec(&["nerdctl", "pull", &full_name])?;

        // Get image metadata
        let inspect_output = self.lima_exec(&["nerdctl", "image", "inspect", &full_name])?;
        let inspect_data: Vec<NerdctlImageInspect> = serde_json::from_str(&inspect_output)
            .map_err(|e| anyhow!("Failed to parse image metadata: {e}"))?;

        let image_data = inspect_data
            .first()
            .ok_or_else(|| anyhow!("No image data returned".to_string()))?;

        // Extract layer information
        let layers = image_data
            .root_fs
            .as_ref()
            .map(|fs| {
                fs.layers
                    .iter()
                    .map(|layer_id| crate::domain::LayerInfo {
                        digest: layer_id.clone(),
                        size: image_data.size.unwrap_or(0) as u64 / fs.layers.len() as u64,
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(ImageMetadata {
            name: name.to_string(),
            tag: tag.to_string(),
            layers,
        })
    }

    fn get_image_layers(&self, name: &str, tag: &str) -> Result<Vec<PathBuf>> {
        let full_name = format!("{name}:{tag}");
        // Use a path under /tmp — Lima mounts /tmp into the VM, so these
        // paths are accessible from both the macOS host and the Lima VM.
        let safe_name = name.replace('/', "-");
        let export_base = format!("/tmp/minibox-layers/{safe_name}/{tag}");

        let inspect_output = self.lima_exec(&["nerdctl", "image", "inspect", &full_name])?;
        let inspect_data: Vec<NerdctlImageInspect> = serde_json::from_str(&inspect_output)
            .map_err(|e| anyhow!("Failed to parse image metadata: {e}"))?;

        let image_data = inspect_data
            .first()
            .ok_or_else(|| anyhow!("No image data returned"))?;

        let layer_ids = image_data
            .root_fs
            .as_ref()
            .map(|fs| fs.layers.clone())
            .unwrap_or_default();

        if layer_ids.is_empty() {
            return Ok(Vec::new());
        }

        // Export the image to the shared /tmp location and unpack it.
        // `nerdctl save` produces a Docker-format tar: each layer is a
        // directory containing layer.tar.  We extract the outer tar and
        // then extract each layer.tar into a rootfs/ subdirectory.
        let tar_path = format!("{export_base}.tar");
        self.lima_exec(&[
            "sh",
            "-c",
            &format!(
                "mkdir -p {export_base} && nerdctl save {full_name} -o {tar_path} && tar xf {tar_path} -C {export_base}"
            ),
        ])?;

        // Build one host-accessible path per layer.
        let layer_paths = layer_ids
            .iter()
            .map(|layer_id| {
                // Use the first 12 hex chars of the digest as the directory name.
                let short_id = layer_id
                    .trim_start_matches("sha256:")
                    .chars()
                    .take(12)
                    .collect::<String>();
                let layer_dir = format!("{export_base}/{short_id}");
                // Extract the layer tar into a rootfs/ subdirectory inside the VM.
                let _ = self.lima_exec(&[
                    "sh",
                    "-c",
                    &format!("mkdir -p {layer_dir}/rootfs && tar xf {layer_dir}/layer.tar -C {layer_dir}/rootfs 2>/dev/null || true"),
                ]);
                PathBuf::from(format!("{layer_dir}/rootfs"))
            })
            .collect();

        Ok(layer_paths)
    }
}

// ============================================================================
// Colima Filesystem Adapter
// ============================================================================

/// Colima adapter for FilesystemProvider trait.
///
/// Delegates overlay filesystem operations to Lima VM.
pub struct ColimaFilesystem {
    instance: String,
    limactl_path: String,
    executor: Option<LimaExecutor>,
}

impl ColimaFilesystem {
    pub fn new() -> Self {
        Self {
            instance: "colima".to_string(),
            limactl_path: "limactl".to_string(),
            executor: None,
        }
    }

    fn lima_exec(&self, args: &[&str]) -> Result<String> {
        if let Some(exec) = &self.executor {
            return exec(args);
        }
        let output = Command::new(&self.limactl_path)
            .arg("shell")
            .arg(&self.instance)
            .args(args)
            .output()
            .map_err(|e| anyhow!("Failed to execute limactl: {e}"))?;

        if !output.status.success() {
            return Err(anyhow!(
                "Lima command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

impl FilesystemProvider for ColimaFilesystem {
    fn setup_rootfs(&self, layers: &[PathBuf], container_dir: &Path) -> Result<PathBuf> {
        // Build overlay mount command
        let lower_dirs = layers
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(":");

        let upper_dir = container_dir.join("upper");
        let work_dir = container_dir.join("work");
        let merged_dir = container_dir.join("merged");

        // Create directories in Lima VM
        self.lima_exec(&["mkdir", "-p", &upper_dir.to_string_lossy()])?;
        self.lima_exec(&["mkdir", "-p", &work_dir.to_string_lossy()])?;
        self.lima_exec(&["mkdir", "-p", &merged_dir.to_string_lossy()])?;

        // Mount overlay filesystem
        let mount_cmd = format!(
            "mount -t overlay overlay -o lowerdir={},upperdir={},workdir={} {}",
            lower_dirs,
            upper_dir.to_string_lossy(),
            work_dir.to_string_lossy(),
            merged_dir.to_string_lossy()
        );

        self.lima_exec(&["sh", "-c", &mount_cmd])?;

        Ok(merged_dir)
    }

    fn pivot_root(&self, new_root: &Path) -> Result<()> {
        // pivot_root is handled by the container runtime inside the VM
        // This is a no-op for the adapter layer
        let _ = new_root; // Suppress unused warning
        Ok(())
    }

    fn cleanup(&self, container_dir: &Path) -> Result<()> {
        let merged_dir = container_dir.join("merged");

        // Unmount overlay
        self.lima_exec(&["umount", &merged_dir.to_string_lossy()])?;

        // Remove directories
        self.lima_exec(&["rm", "-rf", &container_dir.to_string_lossy()])?;

        Ok(())
    }
}

// ============================================================================
// Colima Resource Limiter Adapter
// ============================================================================

/// Colima adapter for ResourceLimiter trait.
///
/// Delegates cgroup operations to Lima VM.
pub struct ColimaLimiter {
    instance: String,
    limactl_path: String,
    executor: Option<LimaExecutor>,
}

impl ColimaLimiter {
    pub fn new() -> Self {
        Self {
            instance: "colima".to_string(),
            limactl_path: "limactl".to_string(),
            executor: None,
        }
    }

    fn lima_exec(&self, args: &[&str]) -> Result<String> {
        if let Some(exec) = &self.executor {
            return exec(args);
        }
        let output = Command::new(&self.limactl_path)
            .arg("shell")
            .arg(&self.instance)
            .args(args)
            .output()
            .map_err(|e| anyhow!("Failed to execute limactl: {e}"))?;

        if !output.status.success() {
            return Err(anyhow!(
                "Lima command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

impl ResourceLimiter for ColimaLimiter {
    fn create(&self, container_id: &str, config: &ResourceConfig) -> Result<String> {
        let cgroup_path = format!("/sys/fs/cgroup/minibox/{container_id}");

        // Create cgroup directory
        self.lima_exec(&["mkdir", "-p", &cgroup_path])?;

        // Set memory limit
        if let Some(memory_bytes) = config.memory_limit_bytes {
            let memory_file = format!("{cgroup_path}/memory.max");
            self.lima_exec(&["sh", "-c", &format!("echo {memory_bytes} > {memory_file}")])?;
        }

        // Set CPU weight
        if let Some(cpu_weight) = config.cpu_weight {
            let cpu_file = format!("{cgroup_path}/cpu.weight");
            self.lima_exec(&["sh", "-c", &format!("echo {cpu_weight} > {cpu_file}")])?;
        }

        // Set PID limit
        if let Some(pids_max) = config.pids_max {
            let pids_file = format!("{cgroup_path}/pids.max");
            self.lima_exec(&["sh", "-c", &format!("echo {pids_max} > {pids_file}")])?;
        }

        // Set I/O limit
        if let Some(io_max) = config.io_max_bytes_per_sec {
            // Format: "major:minor rbps=X wbps=X"
            // This is simplified - production would need device major:minor detection
            let io_file = format!("{cgroup_path}/io.max");
            self.lima_exec(&[
                "sh",
                "-c",
                &format!("echo '8:0 rbps={io_max} wbps={io_max}' > {io_file}"),
            ])?;
        }

        Ok(cgroup_path)
    }

    fn add_process(&self, container_id: &str, pid: u32) -> Result<()> {
        let cgroup_path = format!("/sys/fs/cgroup/minibox/{container_id}");
        let procs_file = format!("{cgroup_path}/cgroup.procs");

        self.lima_exec(&["sh", "-c", &format!("echo {pid} > {procs_file}")])?;

        Ok(())
    }

    fn cleanup(&self, container_id: &str) -> Result<()> {
        let cgroup_path = format!("/sys/fs/cgroup/minibox/{container_id}");

        // Remove cgroup directory
        self.lima_exec(&["rmdir", &cgroup_path])?;

        Ok(())
    }
}

// ============================================================================
// Colima Container Runtime Adapter
// ============================================================================

/// Colima adapter for ContainerRuntime trait.
///
/// Delegates container spawning to Lima VM using containerd/runc.
pub struct ColimaRuntime {
    instance: String,
    limactl_path: String,
    executor: Option<LimaExecutor>,
}

impl ColimaRuntime {
    pub fn new() -> Self {
        Self {
            instance: "colima".to_string(),
            limactl_path: "limactl".to_string(),
            executor: None,
        }
    }

    pub fn with_executor(mut self, executor: LimaExecutor) -> Self {
        self.executor = Some(executor);
        self
    }

    fn lima_exec(&self, args: &[&str]) -> Result<String> {
        if let Some(exec) = &self.executor {
            return exec(args);
        }
        let output = Command::new(&self.limactl_path)
            .arg("shell")
            .arg(&self.instance)
            .args(args)
            .output()
            .map_err(|e| anyhow!("Failed to execute limactl: {e}"))?;

        if !output.status.success() {
            return Err(anyhow!(
                "Lima command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

#[async_trait]
impl ContainerRuntime for ColimaRuntime {
    fn capabilities(&self) -> RuntimeCapabilities {
        // Colima runs a Lima VM with a full Linux kernel — all features available
        RuntimeCapabilities {
            supports_user_namespaces: true,
            supports_cgroups_v2: true,
            supports_overlay_fs: true,
            supports_network_isolation: true,
            max_containers: None,
        }
    }

    async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<u32> {
        // Serialize spawn config to JSON for passing to Lima VM
        let config_json = serde_json::to_string(&SpawnRequest {
            rootfs: config.rootfs.to_string_lossy().to_string(),
            command: config.command.clone(),
            args: config.args.clone(),
            env: config.env.clone(),
            hostname: config.hostname.clone(),
            cgroup_path: config.cgroup_path.to_string_lossy().to_string(),
        })
        .map_err(|e| anyhow!("Failed to serialize config: {e}"))?;

        // Execute container spawn script inside Lima VM.
        // Args are serialised in the JSON config and extracted via jq.
        let spawn_script = format!(
            r#"
            CONFIG='{config_json}'

            ROOTFS=$(echo "$CONFIG" | jq -r '.rootfs')
            COMMAND=$(echo "$CONFIG" | jq -r '.command')
            HOSTNAME=$(echo "$CONFIG" | jq -r '.hostname')
            CGROUP=$(echo "$CONFIG" | jq -r '.cgroup_path')

            # Build args array from JSON
            mapfile -t ARGS < <(echo "$CONFIG" | jq -r '.args[]')

            unshare --pid --mount --uts --ipc --net \
                --fork --kill-child \
                chroot "$ROOTFS" "$COMMAND" "${{ARGS[@]}}" &

            echo $!
            "#
        );

        let output = self.lima_exec(&["sh", "-c", &spawn_script])?;
        let pid: u32 = output
            .trim()
            .parse()
            .map_err(|e| anyhow!("Invalid PID returned: {e}"))?;

        Ok(pid)
    }
}

// ============================================================================
// Helper Types
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct SpawnRequest {
    rootfs: String,
    command: String,
    args: Vec<String>,
    env: Vec<String>,
    hostname: String,
    cgroup_path: String,
}

#[derive(Debug, Deserialize)]
struct NerdctlImageInspect {
    #[serde(rename = "Size")]
    size: Option<i64>,
    #[serde(rename = "RootFS")]
    root_fs: Option<RootFs>,
}

#[derive(Debug, Deserialize)]
struct RootFs {
    #[serde(rename = "Layers")]
    layers: Vec<String>,
}

adapt!(ColimaRegistry, ColimaFilesystem, ColimaLimiter, ColimaRuntime);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_colima_registry_creation() {
        let registry = ColimaRegistry::new();
        assert_eq!(registry.instance, "colima");
    }

    #[test]
    fn test_colima_with_custom_instance() {
        let registry = ColimaRegistry::new().with_instance("custom-lima".to_string());
        assert_eq!(registry.instance, "custom-lima");
    }

    #[test]
    fn test_macos_to_lima_path() {
        let registry = ColimaRegistry::new();

        // Valid paths
        assert!(
            registry
                .macos_to_lima_path(Path::new("/Users/joe/project"))
                .is_ok()
        );
        assert!(registry.macos_to_lima_path(Path::new("/tmp/test")).is_ok());

        // Invalid paths (not mounted)
        assert!(
            registry
                .macos_to_lima_path(Path::new("/var/lib/minibox"))
                .is_err()
        );
    }

    /// Layer paths must live under /tmp or /Users — the Lima-shared mounts
    /// accessible from the macOS host.  Returning /var/lib/containerd/...
    /// gives paths that only exist inside the VM.
    #[test]
    fn get_image_layers_returns_host_accessible_paths() {
        let fake_inspect =
            r#"[{"Size":1024,"RootFS":{"Layers":["sha256:abc123def456","sha256:def456abc789"]}}]"#;

        let registry = ColimaRegistry::new().with_executor(Arc::new(move |args: &[&str]| {
            if args.contains(&"inspect") {
                Ok(fake_inspect.to_string())
            } else {
                Ok(String::new())
            }
        }));

        let layers = registry.get_image_layers("alpine", "latest").unwrap();

        assert_eq!(layers.len(), 2, "should return one path per layer");
        for layer in &layers {
            let s = layer.to_string_lossy();
            assert!(
                s.starts_with("/tmp/") || s.starts_with("/Users/"),
                "layer path {s:?} is not in a Lima-shared directory (/tmp or /Users)"
            );
        }
    }

    /// spawn_process must include config.args in the shell script sent to the
    /// Lima VM.  The current implementation only substitutes $COMMAND and
    /// silently drops all arguments.
    #[tokio::test]
    async fn spawn_process_includes_args_in_script() {
        use crate::domain::ContainerSpawnConfig;
        use std::sync::{Arc, Mutex};

        let captured = Arc::new(Mutex::new(String::new()));
        let cap = captured.clone();

        let runtime = ColimaRuntime::new().with_executor(Arc::new(move |args: &[&str]| {
            // Capture the sh -c script
            if let Some(pos) = args.iter().position(|&a| a == "-c") {
                if let Some(script) = args.get(pos + 1) {
                    *cap.lock().unwrap() = script.to_string();
                }
            }
            Ok("42\n".to_string())
        }));

        let config = ContainerSpawnConfig {
            rootfs: PathBuf::from("/tmp/rootfs"),
            command: "/bin/echo".to_string(),
            args: vec!["hello".to_string(), "world".to_string()],
            env: vec![],
            hostname: "test-container".to_string(),
            cgroup_path: PathBuf::from("/sys/fs/cgroup/minibox/test"),
        };

        let pid = runtime.spawn_process(&config).await.unwrap();
        assert_eq!(pid, 42);

        let script = captured.lock().unwrap().clone();
        assert!(
            script.contains("hello"),
            "spawn script missing arg 'hello': {script}"
        );
        assert!(
            script.contains("world"),
            "spawn script missing arg 'world': {script}"
        );
    }
}
