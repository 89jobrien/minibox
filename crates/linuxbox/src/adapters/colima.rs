//! Colima adapter suite for macOS support via Lima VMs.
//!
//! This module provides four adapters that together implement the full set of
//! domain traits ([`ImageRegistry`], [`FilesystemProvider`], [`ResourceLimiter`],
//! [`ContainerRuntime`]) by delegating operations into a Colima (Lima) Linux VM
//! running on the macOS host.
//!
//! # How it works
//!
//! Each adapter runs commands inside the Lima VM using `limactl shell <instance>
//! <command>`. Lima mounts the macOS `/Users` and `/tmp` trees into the VM, so
//! paths under those prefixes are visible from both sides. Paths outside those
//! prefixes (e.g. `/var/lib/minibox`) exist only inside the VM and cannot be
//! accessed directly from the host.
//!
//! # Adapter selection
//!
//! Selected by `MINIBOX_ADAPTER=colima`. These adapters are compiled on all
//! platforms but are only wired into `miniboxd` on macOS. They are **not** yet
//! listed in the daemon's `MINIBOX_ADAPTER` switch; they are library-only for now.
//!
//! # Requirements
//!
//! - Colima installed (`brew install colima`)
//! - A running Colima VM (`colima start`)
//! - `nerdctl` and `jq` available inside the VM

use crate::adapt;
use crate::domain::{
    ContainerRuntime, ContainerSpawnConfig, FilesystemProvider, ImageMetadata, ImageRegistry,
    ResourceConfig, ResourceLimiter, RuntimeCapabilities, SpawnResult,
};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn colima_home() -> PathBuf {
    if let Ok(path) = std::env::var("COLIMA_HOME")
        && !path.is_empty()
    {
        return PathBuf::from(path);
    }

    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("~"))
        .join(".colima")
}

fn lima_home() -> String {
    if let Ok(path) = std::env::var("LIMA_HOME")
        && !path.is_empty()
    {
        return path;
    }

    colima_home().join("_lima").to_string_lossy().to_string()
}

fn limactl_command(path: &str) -> Command {
    let mut cmd = Command::new(path);
    cmd.env("LIMA_HOME", lima_home());
    cmd
}

/// Callable that runs a command inside the Lima VM and returns its stdout.
///
/// The default implementation invokes `limactl shell <instance> <args…>`.
/// Tests inject a fake closure via [`ColimaRegistry::with_executor`] /
/// [`ColimaRuntime::with_executor`] to avoid real `limactl` calls.
pub type LimaExecutor = Arc<dyn Fn(&[&str]) -> Result<String> + Send + Sync>;

/// Callable that starts a long-lived process inside the Lima VM,
/// returning the [`Child`](std::process::Child) handle with piped stdout.
///
/// The default implementation invokes `limactl shell <instance> <args...>`
/// with [`Stdio::piped`](std::process::Stdio::piped) stdout.
/// Tests inject a fake closure via [`ColimaRuntime::with_spawner`] to
/// avoid real `limactl` calls.
#[allow(dead_code)]
pub type LimaSpawner = Arc<dyn Fn(&[&str]) -> Result<std::process::Child> + Send + Sync>;

// ============================================================================
// Colima Image Registry Adapter
// ============================================================================

/// Colima implementation of [`ImageRegistry`].
///
/// Pulls images and inspects layer metadata via `nerdctl` running inside the
/// Colima Lima VM. Returned layer paths are under `/tmp/minibox-layers/…` so
/// they are accessible from the macOS host via Lima's shared `/tmp` mount.
pub struct ColimaRegistry {
    /// Lima instance name (the argument passed to `limactl shell`; usually `"colima"`).
    instance: String,
    /// Path to the `limactl` binary on the macOS host.
    limactl_path: String,
    /// Optional injected executor used in tests to avoid real `limactl` calls.
    executor: Option<LimaExecutor>,
}

impl ColimaRegistry {
    /// Create a new registry adapter targeting the default `"colima"` Lima instance.
    pub fn new() -> Self {
        Self {
            instance: "colima".to_string(),
            limactl_path: "limactl".to_string(),
            executor: None,
        }
    }

    /// Override the Lima instance name (default: `"colima"`).
    ///
    /// Useful when multiple Lima instances are running (e.g. `colima-arm`).
    pub fn with_instance(mut self, instance: String) -> Self {
        self.instance = instance;
        self
    }

    /// Inject a custom executor for testing.
    ///
    /// The closure receives the argument slice that would be passed to
    /// `limactl shell <instance>` and must return the command's stdout.
    pub fn with_executor(mut self, executor: LimaExecutor) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Run a command inside the Lima VM and return its stdout as a `String`.
    ///
    /// If an injected executor is present it is used instead of a real
    /// `limactl` subprocess — this is the test seam.
    fn lima_exec(&self, args: &[&str]) -> Result<String> {
        if let Some(exec) = &self.executor {
            return exec(args);
        }
        let output = limactl_command(&self.limactl_path)
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

    /// Translate a macOS host path to the equivalent path visible inside the Lima VM.
    ///
    /// Lima mounts the macOS `/Users` and `/tmp` trees at the same paths inside
    /// the VM. Paths outside those prefixes are not accessible from the host and
    /// this function returns an error for them.
    #[allow(dead_code)]
    fn macos_to_lima_path(&self, macos_path: &Path) -> Result<String> {
        let path_str = macos_path
            .to_str()
            .ok_or_else(|| anyhow!("Invalid path encoding".to_string()))?;

        // Lima VM mirrors /Users and /tmp from the macOS host.
        if path_str.starts_with("/Users/") || path_str.starts_with("/tmp/") {
            Ok(path_str.to_string())
        } else {
            Err(anyhow!(
                "Path not mounted in Lima VM: {path_str}. Only /Users and /tmp are typically mounted."
            ))
        }
    }
}

#[async_trait]
impl ImageRegistry for ColimaRegistry {
    /// Return `true` if the image is present in the containerd image store inside the VM.
    ///
    /// Runs `nerdctl image inspect <name>:<tag>` and treats a non-zero exit code as absent.
    async fn has_image(&self, name: &str, tag: &str) -> bool {
        let full_name = format!("{name}:{tag}");
        let result = self.lima_exec(&["nerdctl", "image", "inspect", &full_name]);
        result.is_ok()
    }

    /// Pull the image via `nerdctl` inside the VM and return its metadata.
    ///
    /// Layer sizes in the returned [`ImageMetadata`] are approximate: the total
    /// image size reported by `nerdctl image inspect` is divided equally among
    /// the layers because the per-layer compressed size is not surfaced by the
    /// inspect output.
    ///
    /// # Errors
    ///
    /// Returns an error if `nerdctl pull` or `nerdctl image inspect` fail inside
    /// the VM, or if the inspect JSON cannot be parsed.
    async fn pull_image(&self, name: &str, tag: &str) -> Result<ImageMetadata> {
        let full_name = format!("{name}:{tag}");

        // Pull image using nerdctl inside the Colima VM.
        self.lima_exec(&["nerdctl", "pull", &full_name])?;

        // Retrieve layer information from the containerd image store.
        let inspect_output = self.lima_exec(&["nerdctl", "image", "inspect", &full_name])?;
        let inspect_data: Vec<NerdctlImageInspect> = serde_json::from_str(&inspect_output)
            .map_err(|e| anyhow!("Failed to parse image metadata: {e}"))?;

        let image_data = inspect_data
            .first()
            .ok_or_else(|| anyhow!("No image data returned".to_string()))?;

        // Build LayerInfo list from the RootFS layer digest list.
        // Size is approximated as (total image size / layer count).
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

    /// Export the image from containerd and return host-accessible layer paths.
    ///
    /// Uses `nerdctl save` to export the image as a Docker-format tar, then
    /// extracts each layer into `/tmp/minibox-layers/<name>/<tag>/<short-digest>/rootfs/`.
    /// The `/tmp` prefix is chosen because Lima mounts it into the VM, making
    /// the paths accessible from the macOS host.
    ///
    /// # Errors
    ///
    /// Returns an error if the image is not present in the containerd store or
    /// if the extraction commands fail inside the VM.
    fn get_image_layers(&self, name: &str, tag: &str) -> Result<Vec<PathBuf>> {
        let full_name = format!("{name}:{tag}");
        // Use /tmp — Lima mounts /tmp into the VM, so these paths are
        // accessible from both the macOS host and inside the Lima VM.
        let safe_name = name.replace('/', "-");
        let export_base = format!("/tmp/minibox-layers/{safe_name}/{tag}");

        // Export the image to the shared /tmp location and unpack the outer tar.
        // `nerdctl save` produces a Docker-format tar where each layer is a
        // directory containing `layer.tar`. We parse `manifest.json` to locate
        // those layer tarballs rather than guessing directory names.
        let tar_path = format!("{export_base}.tar");
        self.lima_exec(&[
            "sh",
            "-c",
            &format!(
                "mkdir -p {export_base} && nerdctl save {full_name} -o {tar_path} && tar xf {tar_path} -C {export_base}"
            ),
        ])?;

        let manifest_output = self.lima_exec(&["cat", &format!("{export_base}/manifest.json")])?;
        let manifest: Vec<DockerSaveManifestEntry> = serde_json::from_str(&manifest_output)
            .map_err(|e| anyhow!("Failed to parse exported image manifest: {e}"))?;
        let layer_paths = manifest
            .first()
            .map(|entry| entry.layers.clone())
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(index, layer_tar_rel)| {
                let layer_tar = format!("{export_base}/{layer_tar_rel}");
                let layer_rootfs = format!("{export_base}/rootfs-{index}");
                // Use two separate argv-style calls instead of sh -c to avoid
                // command injection via shell metacharacters in manifest-provided paths.
                let _ = self.lima_exec(&["mkdir", "-p", &layer_rootfs]);
                let _ = self.lima_exec(&["tar", "xf", &layer_tar, "-C", &layer_rootfs]);
                PathBuf::from(layer_rootfs)
            })
            .collect();

        Ok(layer_paths)
    }
}

// ============================================================================
// Colima Filesystem Adapter
// ============================================================================

/// Colima implementation of [`FilesystemProvider`].
///
/// Sets up and tears down overlay mounts inside the Lima VM by running
/// `mount`/`umount` commands via `limactl shell`. The container directory
/// must be under `/tmp` or `/Users` so it is visible from both the host and
/// the VM.
pub struct ColimaFilesystem {
    /// Lima instance name.
    instance: String,
    /// Path to the `limactl` binary on the macOS host.
    limactl_path: String,
    /// Optional injected executor used in tests.
    executor: Option<LimaExecutor>,
}

impl ColimaFilesystem {
    /// Create a new filesystem adapter targeting the default `"colima"` Lima instance.
    pub fn new() -> Self {
        Self {
            instance: "colima".to_string(),
            limactl_path: "limactl".to_string(),
            executor: None,
        }
    }

    /// Inject a custom executor for testing.
    ///
    /// The closure receives the argument slice that would be passed to
    /// `limactl shell <instance>` and must return the command's stdout.
    pub fn with_executor(mut self, executor: LimaExecutor) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Run a command inside the Lima VM and return its stdout as a `String`.
    fn lima_exec(&self, args: &[&str]) -> Result<String> {
        if let Some(exec) = &self.executor {
            return exec(args);
        }
        let output = limactl_command(&self.limactl_path)
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
    /// Create an overlay mount inside the Lima VM and return the merged directory path.
    ///
    /// Creates `upper/`, `work/`, and `merged/` subdirectories under
    /// `container_dir`, then mounts an overlay filesystem with the provided
    /// layer paths as the read-only lower directories.
    ///
    /// # Errors
    ///
    /// Returns an error if any `mkdir -p` or `mount -t overlay` command fails
    /// inside the VM (e.g. insufficient privileges or kernel module not loaded).
    fn setup_rootfs(&self, layers: &[PathBuf], container_dir: &Path) -> Result<PathBuf> {
        // Concatenate all layer paths as colon-separated lowerdir value.
        let lower_dirs = layers
            .iter()
            .rev()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(":");

        let upper_dir = container_dir.join("upper");
        let work_dir = container_dir.join("work");
        let merged_dir = container_dir.join("merged");

        // Create the overlay support directories inside the VM.
        self.lima_exec(&["mkdir", "-p", &upper_dir.to_string_lossy()])?;
        self.lima_exec(&["mkdir", "-p", &work_dir.to_string_lossy()])?;
        self.lima_exec(&["mkdir", "-p", &merged_dir.to_string_lossy()])?;

        // Mount the overlay filesystem inside the VM. Pass argv directly so
        // host paths such as "~/Library/Application Support/..." are handled
        // safely without shell-quoting bugs.
        let mount_opts = format!(
            "lowerdir={},upperdir={},workdir={}",
            lower_dirs,
            upper_dir.to_string_lossy(),
            work_dir.to_string_lossy(),
        );

        self.lima_exec(&[
            "sudo",
            "mount",
            "-t",
            "overlay",
            "overlay",
            "-o",
            &mount_opts,
            &merged_dir.to_string_lossy(),
        ])?;

        Ok(merged_dir)
    }

    /// No-op: `pivot_root` is handled by the container runtime inside the VM.
    ///
    /// In the Colima adapter the actual `pivot_root(2)` call is performed by
    /// the `unshare`/`chroot` invocation in [`ColimaRuntime::spawn_process`],
    /// not by the filesystem provider layer.
    fn pivot_root(&self, new_root: &Path) -> Result<()> {
        let _ = new_root;
        Ok(())
    }

    /// Unmount the overlay and remove the container directory inside the VM.
    ///
    /// # Errors
    ///
    /// Returns an error if `umount` or `rm -rf` fail inside the VM.
    fn cleanup(&self, container_dir: &Path) -> Result<()> {
        let merged_dir = container_dir.join("merged");

        // Unmount the overlay before removing the directory tree.
        self.lima_exec(&["sudo", "umount", &merged_dir.to_string_lossy()])?;
        self.lima_exec(&["rm", "-rf", &container_dir.to_string_lossy()])?;

        Ok(())
    }
}

// ============================================================================
// Colima Resource Limiter Adapter
// ============================================================================

/// Colima implementation of [`ResourceLimiter`].
///
/// Creates and tears down cgroups v2 directories inside the Lima VM by
/// writing directly to `/sys/fs/cgroup/minibox/<container_id>/` via shell
/// commands. The VM's Linux kernel manages the actual resource accounting.
///
/// Note: the I/O limit (`io.max`) uses a hardcoded device number of `8:0`
/// (the conventional major:minor for the first SCSI disk). Colima VMs backed
/// by virtio block devices (`vda` = `253:0`) will have the write silently
/// ignored by the kernel. A future improvement would detect the correct device
/// by reading `/sys/block/*/dev` inside the VM.
pub struct ColimaLimiter {
    /// Lima instance name.
    instance: String,
    /// Path to the `limactl` binary on the macOS host.
    limactl_path: String,
    /// Optional injected executor used in tests.
    executor: Option<LimaExecutor>,
    /// Block device major:minor detected from the VM (e.g. "253:0" for virtio).
    /// Probed once in `with_executor`; used for io.max writes.
    block_device: Option<String>,
}

impl ColimaLimiter {
    /// Create a new resource limiter adapter targeting the default `"colima"` Lima instance.
    pub fn new() -> Self {
        Self {
            instance: "colima".to_string(),
            limactl_path: "limactl".to_string(),
            executor: None,
            block_device: None,
        }
    }

    /// Inject a custom executor and probe the VM's block device for io.max.
    ///
    /// Block device detection is best-effort: if the probe fails or returns an
    /// unexpected format, `block_device` stays `None` and io.max writes are
    /// silently skipped (matching GkeLimiter's best-effort behavior).
    pub fn with_executor(mut self, executor: LimaExecutor) -> Self {
        // Probe block device — best-effort, io.max is optional.
        self.block_device = executor(&[
            "sh",
            "-c",
            "cat $(ls /sys/block/*/dev | head -1) 2>/dev/null",
        ])
        .ok()
        .and_then(|s| {
            let trimmed = s.trim().to_string();
            if trimmed.contains(':') {
                Some(trimmed)
            } else {
                None
            }
        });
        if self.block_device.is_none() {
            tracing::warn!(
                "colima: no block device detected in VM — io.max writes will be skipped"
            );
        }
        self.executor = Some(executor);
        self
    }

    /// Run a command inside the Lima VM and return its stdout as a `String`.
    fn lima_exec(&self, args: &[&str]) -> Result<String> {
        if let Some(exec) = &self.executor {
            return exec(args);
        }
        let output = limactl_command(&self.limactl_path)
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
    /// Create a cgroup for `container_id` and apply the requested resource limits.
    ///
    /// Creates `/sys/fs/cgroup/minibox/<container_id>/` inside the VM and
    /// writes to the relevant cgroup v2 control files:
    /// - `memory.max` if `config.memory_limit_bytes` is set
    /// - `cpu.weight` if `config.cpu_weight` is set
    /// - `pids.max` if `config.pids_max` is set
    /// - `io.max` if `config.io_max_bytes_per_sec` is set (uses device `8:0`)
    ///
    /// Returns the cgroup path string (`/sys/fs/cgroup/minibox/<container_id>`).
    ///
    /// # Errors
    ///
    /// Returns an error if `mkdir` or any cgroup control-file write fails inside the VM.
    fn create(&self, container_id: &str, config: &ResourceConfig) -> Result<String> {
        let parent_cgroup = "/sys/fs/cgroup/minibox";
        let cgroup_path = format!("/sys/fs/cgroup/minibox/{container_id}");

        self.lima_exec(&["sudo", "mkdir", "-p", parent_cgroup])?;
        // Writing to subtree_control fails with EBUSY on cgroup v2 once child
        // cgroups already exist (kernel restriction). Ignore the error so that
        // the second and subsequent container creations succeed.
        let _ = self.lima_exec(&[
            "sudo",
            "sh",
            "-c",
            &format!("echo +cpu +memory +pids +io > {parent_cgroup}/cgroup.subtree_control 2>/dev/null || true"),
        ]);
        self.lima_exec(&["sudo", "mkdir", "-p", &cgroup_path])?;

        if let Some(memory_bytes) = config.memory_limit_bytes {
            let memory_file = format!("{cgroup_path}/memory.max");
            self.lima_exec(&[
                "sudo",
                "sh",
                "-c",
                &format!("echo {memory_bytes} > {memory_file}"),
            ])?;
        }

        if let Some(cpu_weight) = config.cpu_weight {
            let cpu_file = format!("{cgroup_path}/cpu.weight");
            self.lima_exec(&[
                "sudo",
                "sh",
                "-c",
                &format!("echo {cpu_weight} > {cpu_file}"),
            ])?;
        }

        if let Some(pids_max) = config.pids_max {
            let pids_file = format!("{cgroup_path}/pids.max");
            self.lima_exec(&[
                "sudo",
                "sh",
                "-c",
                &format!("echo {pids_max} > {pids_file}"),
            ])?;
        }

        // Set I/O limit — requires a detected block device major:minor.
        // If no device was detected at construction time, skip silently.
        if let Some(io_max) = config.io_max_bytes_per_sec
            && let Some(device) = self.block_device.as_deref()
        {
            let io_file = format!("{cgroup_path}/io.max");
            self.lima_exec(&[
                "sudo",
                "sh",
                "-c",
                &format!("echo '{device} rbps={io_max} wbps={io_max}' > {io_file}"),
            ])?;
        }

        Ok(cgroup_path)
    }

    /// Add `pid` to the cgroup associated with `container_id`.
    ///
    /// Writes the PID to `cgroup.procs` inside the VM.
    ///
    /// # Errors
    ///
    /// Returns an error if the write fails (e.g. the cgroup does not exist yet).
    fn add_process(&self, container_id: &str, pid: u32) -> Result<()> {
        let cgroup_path = format!("/sys/fs/cgroup/minibox/{container_id}");
        let procs_file = format!("{cgroup_path}/cgroup.procs");

        self.lima_exec(&["sudo", "sh", "-c", &format!("echo {pid} > {procs_file}")])?;

        Ok(())
    }

    /// Remove the cgroup directory for `container_id` inside the VM.
    ///
    /// Uses `rmdir` rather than `rm -rf` because the kernel rejects removal of
    /// a cgroup that still has attached processes, surfacing the error to the caller.
    ///
    /// # Errors
    ///
    /// Returns an error if `rmdir` fails inside the VM.
    fn cleanup(&self, container_id: &str) -> Result<()> {
        let cgroup_path = format!("/sys/fs/cgroup/minibox/{container_id}");

        // rmdir (not rm -rf): the kernel rejects removal of a non-empty cgroup.
        self.lima_exec(&["sudo", "rmdir", &cgroup_path])?;

        Ok(())
    }
}

// ============================================================================
// Colima Container Runtime Adapter
// ============================================================================

/// Colima implementation of [`ContainerRuntime`].
///
/// Spawns container processes inside the Lima VM using `unshare` + `chroot`.
/// The spawn script is executed via `limactl shell` and the VM PID is returned
/// to the host for tracking and reaping.
pub struct ColimaRuntime {
    /// Lima instance name.
    instance: String,
    /// Path to the `limactl` binary on the macOS host.
    limactl_path: String,
    /// Optional injected executor used in tests.
    executor: Option<LimaExecutor>,
    spawner: Option<LimaSpawner>,
}

impl ColimaRuntime {
    /// Create a new runtime adapter targeting the default `"colima"` Lima instance.
    pub fn new() -> Self {
        Self {
            instance: "colima".to_string(),
            limactl_path: "limactl".to_string(),
            executor: None,
            spawner: None,
        }
    }

    /// Inject a custom executor for testing.
    ///
    /// The closure receives the argument slice that would be passed to
    /// `limactl shell <instance>` and must return the command's stdout.
    pub fn with_executor(mut self, executor: LimaExecutor) -> Self {
        self.executor = Some(executor);
        self
    }

    pub fn with_spawner(mut self, spawner: LimaSpawner) -> Self {
        self.spawner = Some(spawner);
        self
    }

    /// Run a command inside the Lima VM and return its stdout as a `String`.
    fn lima_exec(&self, args: &[&str]) -> Result<String> {
        if let Some(exec) = &self.executor {
            return exec(args);
        }
        let output = limactl_command(&self.limactl_path)
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

    fn lima_spawn(&self, args: &[&str]) -> Result<std::process::Child> {
        if let Some(spawner) = &self.spawner {
            return spawner(args);
        }
        limactl_command(&self.limactl_path)
            .arg("shell")
            .arg(&self.instance)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn limactl: {e}"))
    }
}

#[async_trait]
impl ContainerRuntime for ColimaRuntime {
    /// Return the runtime capabilities advertised by this adapter.
    ///
    /// Colima runs a full Linux kernel inside the Lima VM, so all namespace
    /// and cgroup features are available. These values reflect the VM's
    /// capabilities, not those of the macOS host.
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            supports_user_namespaces: true,
            supports_cgroups_v2: true,
            supports_overlay_fs: true,
            supports_network_isolation: true,
            max_containers: None,
        }
    }

    /// Spawn a container process inside the Lima VM and return its PID.
    ///
    /// Serialises the spawn configuration to JSON, embeds it in a shell script
    /// executed via `limactl shell`, and parses the PID printed to stdout by
    /// the backgrounded `unshare`/`chroot` invocation.
    ///
    /// The `output_reader` field of the returned [`SpawnResult`] is always
    /// `None` — output streaming from Lima-hosted containers is not yet
    /// implemented.
    ///
    /// # Errors
    ///
    /// Returns an error if the `limactl shell` command fails or if the PID
    /// printed to stdout cannot be parsed as a `u32`.
    async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<SpawnResult> {
        // Serialise the spawn config so it can be passed into the shell script
        // as a single JSON string, avoiding shell quoting problems.
        let config_json = serde_json::to_string(&SpawnRequest {
            rootfs: config.rootfs.to_string_lossy().to_string(),
            command: config.command.clone(),
            args: config.args.clone(),
            env: config.env.clone(),
            hostname: config.hostname.clone(),
            cgroup_path: config.cgroup_path.to_string_lossy().to_string(),
        })
        .map_err(|e| anyhow!("Failed to serialize config: {e}"))?;

        if self.spawner.is_some() && config.capture_output {
            // Streaming path: foreground exec with piped stdout.
            // Uses `exec unshare` so the spawned process replaces the shell,
            // making child.id() the container init PID directly.
            let spawn_script = format!(
                r#"CONFIG='{config_json}'
ROOTFS=$(echo "$CONFIG" | jq -r '.rootfs')
COMMAND=$(echo "$CONFIG" | jq -r '.command')
HOSTNAME=$(echo "$CONFIG" | jq -r '.hostname')
mapfile -t ARGS < <(echo "$CONFIG" | jq -r '.args[]')
exec sudo unshare --pid --mount --uts --ipc --net \
    --fork --kill-child \
    chroot "$ROOTFS" "$COMMAND" "${{ARGS[@]}}"
"#
            );

            let mut child = self.lima_spawn(&["sh", "-c", &spawn_script])?;
            let pid = child.id();

            // Take the stdout pipe as an OwnedFd before dropping child.
            #[cfg(unix)]
            let output_reader = {
                let stdout = child
                    .stdout
                    .take()
                    .ok_or_else(|| anyhow!("child stdout pipe missing"))?;
                std::os::fd::OwnedFd::from(stdout)
            };

            #[cfg(not(unix))]
            let output_reader: Option<std::convert::Infallible> = None;

            // INVARIANT: The daemon process is the direct parent of this Child
            // (it called Command::spawn). waitpid(pid) in the reaper will succeed.
            // On Unix, Child::drop does NOT kill the process — it only closes
            // remaining stdio handles (all None after take).
            drop(child);

            tracing::info!(
                pid = pid,
                rootfs = %config.rootfs.display(),
                "colima: spawned foreground container process with piped output"
            );

            return Ok(SpawnResult {
                pid,
                #[cfg(unix)]
                output_reader: Some(output_reader),
                #[cfg(not(unix))]
                output_reader,
            });
        }

        // Shell script executed inside the Lima VM.
        // Uses `jq` to extract fields from the JSON config, then runs
        // `unshare` with Linux namespace flags to isolate the container.
        let spawn_script = format!(
            r#"
            CONFIG='{config_json}'

            ROOTFS=$(echo "$CONFIG" | jq -r '.rootfs')
            COMMAND=$(echo "$CONFIG" | jq -r '.command')
            HOSTNAME=$(echo "$CONFIG" | jq -r '.hostname')
            CGROUP=$(echo "$CONFIG" | jq -r '.cgroup_path')

            # Build args array from JSON
            mapfile -t ARGS < <(echo "$CONFIG" | jq -r '.args[]')

            sudo unshare --pid --mount --uts --ipc --net \
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

        Ok(SpawnResult {
            pid,
            output_reader: None,
        })
    }
}

// ============================================================================
// Helper Types
// ============================================================================

/// JSON payload passed to the Lima VM's spawn shell script.
#[derive(Debug, Serialize, Deserialize)]
struct SpawnRequest {
    rootfs: String,
    command: String,
    args: Vec<String>,
    env: Vec<String>,
    hostname: String,
    cgroup_path: String,
}

/// Deserialised subset of `nerdctl image inspect` output.
#[derive(Debug, Deserialize)]
struct NerdctlImageInspect {
    /// Total compressed image size in bytes as reported by nerdctl.
    #[serde(rename = "Size")]
    size: Option<i64>,
    /// Layer digest list embedded in the RootFS section.
    #[serde(rename = "RootFS")]
    root_fs: Option<RootFs>,
}

/// The `RootFS` section of `nerdctl image inspect` output.
#[derive(Debug, Deserialize)]
struct RootFs {
    /// Ordered list of layer content digests (e.g. `"sha256:abc123…"`).
    #[serde(rename = "Layers")]
    layers: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DockerSaveManifestEntry {
    #[serde(rename = "Layers")]
    layers: Vec<String>,
}

// Register all four Colima adapters with the adapt! macro so they satisfy
// the AsAny + Default bounds required by the daemon's adapter registry.
adapt!(
    ColimaRegistry,
    ColimaFilesystem,
    ColimaLimiter,
    ColimaRuntime
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialises environment-variable mutations across parallel tests.
    // SAFETY: Rust 2024 requires unsafe for set_var/remove_var. The Mutex
    // ensures only one test modifies the environment at a time.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

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
    fn test_lima_home_defaults_to_colima_lima_dir() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let prev_lima = std::env::var("LIMA_HOME").ok();
        let prev_colima = std::env::var("COLIMA_HOME").ok();
        let prev_home = std::env::var("HOME").ok();

        unsafe {
            std::env::remove_var("LIMA_HOME");
            std::env::remove_var("COLIMA_HOME");
            std::env::set_var("HOME", "/tmp/minibox-colima-home");
        }

        let result = lima_home();

        unsafe {
            match prev_lima {
                Some(v) => std::env::set_var("LIMA_HOME", v),
                None => std::env::remove_var("LIMA_HOME"),
            }
            match prev_colima {
                Some(v) => std::env::set_var("COLIMA_HOME", v),
                None => std::env::remove_var("COLIMA_HOME"),
            }
            match prev_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }

        assert_eq!(result, "/tmp/minibox-colima-home/.colima/_lima");
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

        // Invalid paths (not mounted in Lima VM)
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
        let fake_manifest = r#"[{"Layers":["aaa/layer.tar","bbb/layer.tar"]}]"#;

        let registry = ColimaRegistry::new().with_executor(Arc::new(move |args: &[&str]| {
            if args.first() == Some(&"cat") && args[1].ends_with("/manifest.json") {
                Ok(fake_manifest.to_string())
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
        use crate::domain::{ContainerHooks, ContainerSpawnConfig};
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
            capture_output: false,
            hooks: ContainerHooks::default(),
            skip_network_namespace: false,
        };

        let result = runtime.spawn_process(&config).await.unwrap();
        assert_eq!(result.pid, 42);

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

    #[test]
    fn limiter_detects_block_device() {
        let limiter = ColimaLimiter::new().with_executor(Arc::new(|args: &[&str]| {
            let joined = args.join(" ");
            if joined.contains("/sys/block") {
                Ok("253:0\n".to_string())
            } else {
                Ok(String::new())
            }
        }));
        assert_eq!(limiter.block_device.as_deref(), Some("253:0"));
    }

    #[test]
    fn limiter_io_max_uses_detected_device() {
        let commands = Arc::new(std::sync::Mutex::new(Vec::new()));
        let cmds = commands.clone();
        let limiter = ColimaLimiter::new().with_executor(Arc::new(move |args: &[&str]| {
            cmds.lock().expect("lock").push(args.join(" "));
            if args.join(" ").contains("/sys/block") {
                Ok("253:0\n".to_string())
            } else {
                Ok(String::new())
            }
        }));

        let config = crate::domain::ResourceConfig {
            memory_limit_bytes: None,
            cpu_weight: None,
            pids_max: None,
            io_max_bytes_per_sec: Some(1048576),
        };
        limiter
            .create("test-container", &config)
            .expect("create should succeed");

        let all = commands.lock().expect("lock");
        let io_cmd = all
            .iter()
            .find(|c| c.contains("io.max"))
            .expect("should write io.max");
        assert!(
            io_cmd.contains("253:0"),
            "should use detected device, got: {io_cmd}"
        );
    }

    #[tokio::test]
    async fn spawn_process_returns_piped_output() {
        use crate::domain::{ContainerHooks, ContainerSpawnConfig};
        use std::io::Read;

        let runtime = ColimaRuntime::new().with_spawner(Arc::new(|_args: &[&str]| {
            std::process::Command::new("echo")
                .arg("hello from container")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| anyhow::anyhow!("spawn failed: {e}"))
        }));

        let config = ContainerSpawnConfig {
            rootfs: PathBuf::from("/tmp/rootfs"),
            command: "/bin/echo".to_string(),
            args: vec!["hello".to_string()],
            env: vec![],
            hostname: "test".to_string(),
            cgroup_path: PathBuf::from("/sys/fs/cgroup/minibox/test"),
            capture_output: true,
            skip_network_namespace: false,
            hooks: ContainerHooks::default(),
        };

        let result = runtime
            .spawn_process(&config)
            .await
            .expect("spawn should succeed");
        assert!(result.pid > 0, "PID must be positive");
        assert!(
            result.output_reader.is_some(),
            "output_reader must be Some when spawner is set"
        );

        let fd = result.output_reader.expect("output_reader should be Some");
        let mut file = std::fs::File::from(fd);
        let mut output = String::new();
        file.read_to_string(&mut output)
            .expect("should read output");
        assert!(
            output.contains("hello from container"),
            "output was: {output}"
        );
    }
}
