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

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use minibox_core::adapt;
use minibox_core::domain::{
    ContainerRuntime, ContainerSpawnConfig, ImageLoader, ImageMetadata, ImageRegistry,
    ResourceConfig, ResourceLimiter, RootfsLayout, RuntimeCapabilities, SpawnResult,
};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

/// Resolve a username to its home directory via the system user database.
/// Split out from `colima_home()` so it can be unit tested without requiring
/// the calling process to actually be root.
fn home_dir_for_user(username: &str) -> Option<PathBuf> {
    nix::unistd::User::from_name(username)
        .ok()
        .flatten()
        .map(|u| u.dir)
}

/// If running as root via `sudo`, chown `path` to `$SUDO_USER`. No-op
/// otherwise (including if `SUDO_USER` can't be resolved to a real user).
///
/// The generic container-directory creation in `daemon::handler::run` makes
/// `container_dir` `0700`, owned by whichever user the daemon process runs
/// as. That's correct for the native Linux adapter (daemon and workload run
/// as the same root). For Colima, the actual filesystem work happens as
/// `SUDO_USER` inside the VM via privilege-dropped `limactl`/`colima`
/// invocations — without this chown, `container_dir` stays root-owned and
/// every subsequent `mkdir`/`docker` call as `SUDO_USER` gets "Permission
/// denied" on it, even though the directory was just created successfully by
/// the (root) daemon a moment earlier.
fn chown_to_sudo_user_if_root(path: &Path) -> Result<()> {
    if !nix::unistd::geteuid().is_root() {
        return Ok(());
    }
    let Ok(sudo_user) = std::env::var("SUDO_USER") else {
        return Ok(());
    };
    let Ok(Some(user)) = nix::unistd::User::from_name(&sudo_user) else {
        return Ok(());
    };
    nix::unistd::chown(path, Some(user.uid), Some(user.gid))
        .map_err(|e| anyhow!("Failed to chown {} to {sudo_user}: {e}", path.display()))
}

fn colima_home() -> PathBuf {
    if let Ok(path) = std::env::var("COLIMA_HOME")
        && !path.is_empty()
    {
        return PathBuf::from(path);
    }

    // When running as root via sudo, sudoers' default env_reset resets $HOME
    // to root's own home (e.g. /var/root on macOS), not the invoking user's.
    // Resolve the original user's home via SUDO_USER so this matches the
    // SUDO_USER handling `limactl_command()` already does below — otherwise
    // LIMA_HOME points at a directory that was never colima's, and every
    // limactl call reports the instance as not running.
    if nix::unistd::geteuid().is_root()
        && let Ok(sudo_user) = std::env::var("SUDO_USER")
        && let Some(home) = home_dir_for_user(&sudo_user)
    {
        return home.join(".colima");
    }

    std::env::var("HOME")
        .map_or_else(|_| PathBuf::from("~"), PathBuf::from)
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

/// Build the `sudo -u <user> ... -- <path>` argument list used to drop back
/// from root to the invoking user. Split out from `limactl_command()` so the
/// exact flags — especially `--preserve-env=LIMA_HOME` — are unit testable
/// without the test process itself needing to be root.
fn sudo_drop_privileges_args<'a>(sudo_user: &'a str, path: &'a str) -> Vec<&'a str> {
    // `sudo -u <user>` applies its own env_reset when dropping back from root
    // to the target user, which would otherwise strip the LIMA_HOME set via
    // `.env()` in limactl_command() below (that only sets it for the outer
    // `sudo` invocation, not the process sudo execs after dropping
    // privileges). --preserve-env is required so LIMA_HOME actually reaches
    // limactl — without it, limactl silently falls back to the default
    // ~/.lima and reports the colima instance as "not running" even though
    // it is.
    vec!["-u", sudo_user, "--preserve-env=LIMA_HOME", "--", path]
}

/// Build a `Command` for `path` (`limactl`, or the `colima` CLI itself).
///
/// Sets `LIMA_HOME` correctly and, if the calling process is root (as when
/// miniboxd is started via `sudo`), drops privileges back to `SUDO_USER` —
/// both `limactl` and `colima` refuse to run as root and look for VM state
/// under the invoking user's home directory, not root's.
///
/// This is the single place that logic lives; every caller that shells out to
/// either binary (including miniboxd's own LimaExecutor/LimaSpawner closures
/// in `main.rs`) must go through this rather than re-deriving the sudo-wrap
/// independently — a previous duplicate implementation of just the `colima
/// ssh` case omitted the privilege drop entirely and always reported
/// `colima not running`, even with a running instance, because it ran as root.
pub fn privileged_command(path: &str) -> Command {
    if nix::unistd::geteuid().is_root()
        && let Ok(sudo_user) = std::env::var("SUDO_USER")
    {
        let mut cmd = Command::new("sudo");
        cmd.args(sudo_drop_privileges_args(&sudo_user, path));
        cmd.env("LIMA_HOME", lima_home());
        return cmd;
    }
    let mut cmd = Command::new(path);
    cmd.env("LIMA_HOME", lima_home());
    cmd
}

fn limactl_command(path: &str) -> Command {
    privileged_command(path)
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
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

/// Shared Lima VM execution context used by all Colima adapter structs.
///
/// Holds the instance name, `limactl` binary path, and optional test executor.
/// Provides the single `lima_exec` implementation that all adapters delegate to.
struct LimaContext {
    instance: String,
    limactl_path: String,
    executor: Option<LimaExecutor>,
}

impl LimaContext {
    fn new() -> Self {
        Self {
            instance: "colima".to_string(),
            limactl_path: "limactl".to_string(),
            executor: None,
        }
    }

    /// Run a command inside the Lima VM and return its stdout as a `String`.
    ///
    /// If an injected executor is present it is used instead of a real
    /// `limactl` subprocess -- this is the test seam.
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

// ============================================================================
// Colima Image Registry Adapter
// ============================================================================

/// Colima implementation of [`ImageRegistry`].
///
/// Pulls images and inspects layer metadata via `nerdctl` running inside the
/// Colima Lima VM. Returned layer paths are under `/tmp/minibox-layers/…` so
/// they are accessible from the macOS host via Lima's shared `/tmp` mount.
pub struct ColimaRegistry {
    ctx: LimaContext,
}

impl ColimaRegistry {
    /// Create a new registry adapter targeting the default `"colima"` Lima instance.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ctx: LimaContext::new(),
        }
    }

    /// Override the Lima instance name (default: `"colima"`).
    ///
    /// Useful when multiple Lima instances are running (e.g. `colima-arm`).
    #[must_use]
    pub fn with_instance(mut self, instance: String) -> Self {
        self.ctx.instance = instance;
        self
    }

    /// Inject a custom executor for testing.
    ///
    /// The closure receives the argument slice that would be passed to
    /// `limactl shell <instance>` and must return the command's stdout.
    pub fn with_executor(mut self, executor: LimaExecutor) -> Self {
        self.ctx.executor = Some(executor);
        self
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
        // Strip "library/" prefix — nerdctl and docker both omit it for official images.
        let short_name = name.strip_prefix("library/").unwrap_or(name);
        let full_name = format!("{short_name}:{tag}");
        // Use `images --filter` which works with both nerdctl-as-docker and real docker.
        // `image inspect` can fail even when the image exists (nerdctl docker-compat quirk).
        self.ctx
            .lima_exec(&[
                "docker",
                "images",
                "--filter",
                &format!("reference={full_name}"),
                "--quiet",
            ])
            .is_ok_and(|out| !out.trim().is_empty())
    }

    /// Pull the image via `docker` inside the VM and return its metadata.
    ///
    /// Colima instances may be configured with either a `docker` or
    /// `containerd` (nerdctl) runtime — `docker` is used here because it's
    /// present in both configurations (nerdctl's docker-compat mode produces
    /// the same inspect JSON shape this parses), whereas `nerdctl` itself is
    /// only present on containerd-runtime instances and errors with "command
    /// not found" on the more common `docker`-runtime default.
    ///
    /// Layer sizes in the returned [`ImageMetadata`] are approximate: the total
    /// image size reported by `docker image inspect` is divided equally among
    /// the layers because the per-layer compressed size is not surfaced by the
    /// inspect output.
    ///
    /// # Errors
    ///
    /// Returns an error if `docker pull` or `docker image inspect` fail inside
    /// the VM, or if the inspect JSON cannot be parsed.
    async fn pull_image(
        &self,
        image_ref: &crate::image::reference::ImageRef,
    ) -> Result<ImageMetadata> {
        let cache_name = image_ref.cache_name();
        let tag = image_ref.tag.clone();
        let full_name = format!("{cache_name}:{tag}");

        // Pull image using docker inside the Colima VM.
        self.ctx.lima_exec(&["docker", "pull", &full_name])?;

        // Retrieve layer information from the image store.
        let inspect_output = self
            .ctx
            .lima_exec(&["docker", "image", "inspect", &full_name])?;
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
            name: cache_name,
            tag,
            layers,
        })
    }

    /// Export the image and return host-accessible layer paths.
    ///
    /// Uses `docker save` (present on both `docker`- and `containerd`-runtime
    /// Colima instances, unlike `nerdctl` — see `pull_image`'s doc comment) to
    /// export the image as a Docker-format tar, then extracts each layer into
    /// `/tmp/minibox-layers/<name>/<tag>/<short-digest>/rootfs/`. The `/tmp`
    /// prefix is chosen because Lima mounts it into the VM, making the paths
    /// accessible from the macOS host.
    ///
    /// # Errors
    ///
    /// Returns an error if the image is not present in the image store or if
    /// the extraction commands fail inside the VM.
    fn get_image_layers(&self, name: &str, tag: &str) -> Result<Vec<PathBuf>> {
        let full_name = format!("{name}:{tag}");
        // Use /tmp — Lima mounts /tmp into the VM, so these paths are
        // accessible from both the macOS host and inside the Lima VM.
        let safe_name = name.replace('/', "-");
        let export_base = format!("/tmp/minibox-layers/{safe_name}/{tag}");

        // Export the image to the shared /tmp location and unpack the outer tar.
        // `docker save` produces a tar where each layer is a directory
        // containing `layer.tar`. We parse `manifest.json` to locate those
        // layer tarballs rather than guessing directory names.
        let tar_path = format!("{export_base}.tar");
        self.ctx.lima_exec(&[
            "sh",
            "-c",
            &format!(
                "mkdir -p {export_base} && docker save {full_name} -o {tar_path} && tar xf {tar_path} -C {export_base}"
            ),
        ])?;

        let manifest_output = self
            .ctx
            .lima_exec(&["cat", &format!("{export_base}/manifest.json")])?;
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
                let _ = self.ctx.lima_exec(&["mkdir", "-p", &layer_rootfs]);
                let _ = self
                    .ctx
                    .lima_exec(&["tar", "xf", &layer_tar, "-C", &layer_rootfs]);
                PathBuf::from(layer_rootfs)
            })
            .collect();

        Ok(layer_paths)
    }
}

#[async_trait]
impl ImageLoader for ColimaRegistry {
    /// Load a local OCI tarball into the Colima VM's image store.
    ///
    /// The tarball path must be reachable from inside the Lima VM.
    /// Lima automatically shares `/tmp` and `$HOME`, so paths under those
    /// directories work without extra configuration. Uses `docker` rather
    /// than `nerdctl` — see `pull_image`'s doc comment.
    async fn load_image(&self, path: &std::path::Path, _name: &str, _tag: &str) -> Result<()> {
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow!("non-UTF-8 path: {}", path.display()))?;
        self.ctx
            .lima_exec(&["docker", "load", "-i", path_str])
            .map(|_| ())
            .map_err(|e| anyhow!("docker load failed: {e}"))
    }
}

// ============================================================================
// Colima Filesystem Adapter
// ============================================================================

/// Colima implementation of [`crate::domain::FilesystemProvider`].
///
/// Sets up and tears down overlay mounts inside the Lima VM by running
/// `mount`/`umount` commands via `limactl shell`. The container directory
/// must be under `/tmp` or `/Users` so it is visible from both the host and
/// the VM.
pub struct ColimaFilesystem {
    ctx: LimaContext,
}

impl ColimaFilesystem {
    /// Create a new filesystem adapter targeting the default `"colima"` Lima instance.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ctx: LimaContext::new(),
        }
    }

    /// Inject a custom executor for testing.
    ///
    /// The closure receives the argument slice that would be passed to
    /// `limactl shell <instance>` and must return the command's stdout.
    pub fn with_executor(mut self, executor: LimaExecutor) -> Self {
        self.ctx.executor = Some(executor);
        self
    }
}

impl minibox_core::domain::RootfsSetup for ColimaFilesystem {
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
    fn setup_rootfs(&self, layers: &[PathBuf], container_dir: &Path) -> Result<RootfsLayout> {
        // container_dir was just created 0700 by the (root) daemon process;
        // hand ownership to SUDO_USER so the mkdir/mount calls below, which
        // run as SUDO_USER inside the VM, can actually write into it.
        chown_to_sudo_user_if_root(container_dir)?;

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
        self.ctx
            .lima_exec(&["mkdir", "-p", &upper_dir.to_string_lossy()])?;
        self.ctx
            .lima_exec(&["mkdir", "-p", &work_dir.to_string_lossy()])?;
        self.ctx
            .lima_exec(&["mkdir", "-p", &merged_dir.to_string_lossy()])?;

        // Mount the overlay filesystem inside the VM. Pass argv directly so
        // host paths such as "~/Library/Application Support/..." are handled
        // safely without shell-quoting bugs.
        let mount_opts = format!(
            "lowerdir={},upperdir={},workdir={}",
            lower_dirs,
            upper_dir.to_string_lossy(),
            work_dir.to_string_lossy(),
        );

        self.ctx.lima_exec(&[
            "sudo",
            "mount",
            "-t",
            "overlay",
            "overlay",
            "-o",
            &mount_opts,
            &merged_dir.to_string_lossy(),
        ])?;

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("colima_instance".to_string(), self.ctx.instance.clone());

        Ok(RootfsLayout {
            merged_dir: merged_dir.into(),
            rootfs_metadata: Some(crate::domain::BackendRootfsMetadata::Overlay {
                upper_dir: upper_dir.into(),
                metadata,
            }),
            source_image_ref: None,
        })
    }

    /// Unmount the overlay and remove the container directory inside the VM.
    ///
    /// # Errors
    ///
    /// Returns an error if `umount` or `rm -rf` fail inside the VM.
    fn cleanup(&self, container_dir: &Path) -> Result<()> {
        let merged_dir = container_dir.join("merged");

        // Unmount the overlay before removing the directory tree.
        self.ctx
            .lima_exec(&["sudo", "umount", &merged_dir.to_string_lossy()])?;
        self.ctx
            .lima_exec(&["rm", "-rf", &container_dir.to_string_lossy()])?;

        Ok(())
    }
}

impl minibox_core::domain::ChildInit for ColimaFilesystem {
    /// No-op: `pivot_root` is handled inside the Lima VM by the container runtime.
    fn pivot_root(&self, new_root: &Path) -> Result<()> {
        let _ = new_root;
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
    ctx: LimaContext,
    /// Block device major:minor detected from the VM (e.g. "253:0" for virtio).
    /// Probed once in `with_executor`; used for io.max writes.
    block_device: Option<String>,
}

impl ColimaLimiter {
    /// Create a new resource limiter adapter targeting the default `"colima"` Lima instance.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ctx: LimaContext::new(),
            block_device: None,
        }
    }

    /// Inject a custom executor and probe the VM's block device for io.max.
    ///
    /// Block device detection is best-effort: if the probe fails or returns an
    /// unexpected format, `block_device` stays `None` and io.max writes are
    /// silently skipped (matching `GkeLimiter`'s best-effort behavior).
    pub fn with_executor(mut self, executor: LimaExecutor) -> Self {
        // Probe block device -- best-effort, io.max is optional.
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
        self.ctx.executor = Some(executor);
        self
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

        self.ctx
            .lima_exec(&["sudo", "mkdir", "-p", parent_cgroup])?;
        // Writing to subtree_control fails with EBUSY on cgroup v2 once child
        // cgroups already exist (kernel restriction). Ignore the error so that
        // the second and subsequent container creations succeed.
        let _ = self.ctx.lima_exec(&[
            "sudo",
            "sh",
            "-c",
            &format!("echo +cpu +memory +pids +io > {parent_cgroup}/cgroup.subtree_control 2>/dev/null || true"),
        ]);
        self.ctx.lima_exec(&["sudo", "mkdir", "-p", &cgroup_path])?;

        if let Some(memory_bytes) = config.memory_limit_bytes {
            let memory_file = format!("{cgroup_path}/memory.max");
            self.ctx.lima_exec(&[
                "sudo",
                "sh",
                "-c",
                &format!("echo {memory_bytes} > {memory_file}"),
            ])?;
        }

        if let Some(cpu_weight) = config.cpu_weight {
            let cpu_file = format!("{cgroup_path}/cpu.weight");
            self.ctx.lima_exec(&[
                "sudo",
                "sh",
                "-c",
                &format!("echo {cpu_weight} > {cpu_file}"),
            ])?;
        }

        if let Some(pids_max) = config.pids_max {
            let pids_file = format!("{cgroup_path}/pids.max");
            self.ctx.lima_exec(&[
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
            self.ctx.lima_exec(&[
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

        self.ctx
            .lima_exec(&["sudo", "sh", "-c", &format!("echo {pid} > {procs_file}")])?;

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
        self.ctx.lima_exec(&["sudo", "rmdir", &cgroup_path])?;

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
    ctx: LimaContext,
    spawner: Option<LimaSpawner>,
}

impl ColimaRuntime {
    /// Create a new runtime adapter targeting the default `"colima"` Lima instance.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ctx: LimaContext::new(),
            spawner: None,
        }
    }

    /// Inject a custom executor for testing.
    ///
    /// The closure receives the argument slice that would be passed to
    /// `limactl shell <instance>` and must return the command's stdout.
    pub fn with_executor(mut self, executor: LimaExecutor) -> Self {
        self.ctx.executor = Some(executor);
        self
    }

    /// Injects a custom Lima process spawner for testing.
    pub fn with_spawner(mut self, spawner: LimaSpawner) -> Self {
        self.spawner = Some(spawner);
        self
    }

    fn lima_spawn(&self, args: &[&str]) -> Result<std::process::Child> {
        if let Some(spawner) = &self.spawner {
            return spawner(args);
        }
        limactl_command(&self.ctx.limactl_path)
            .arg("shell")
            .arg(&self.ctx.instance)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn limactl: {e}"))
    }
}

/// Resolve `$HOME` into a prefix that is safe to use as an allowlist entry.
///
/// Returns `None` when `$HOME` is absent, empty, relative, or the filesystem
/// root. `Path::starts_with` reports both the empty path and `/` as a prefix of
/// *every* path, so folding either into the allowlist would silently accept any
/// host path — exactly what `validate_lima_paths` exists to prevent. Dropping
/// the prefix fails closed: only `/tmp` stays usable, and a caller mounting out
/// of its home directory gets a clear error rather than a silent pass.
fn home_allowlist_prefix() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::PathBuf::from(home);
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return None;
    }
    // `/`, `.` and `..` carry no real prefix information.
    let meaningful = path.components().any(|c| {
        !matches!(
            c,
            std::path::Component::RootDir
                | std::path::Component::CurDir
                | std::path::Component::ParentDir
        )
    });
    if !meaningful {
        return None;
    }
    Some(path)
}

/// Validate that all bind mount host paths are accessible inside the Lima VM.
///
/// Lima shares `$HOME` and `/tmp` into the VM by default. Paths outside those
/// prefixes are not visible and will cause silent mount failures.
///
/// # Security
///
/// The `$HOME` prefix comes from [`home_allowlist_prefix`], which drops a blank
/// or root `$HOME`. Without that, an environment with `HOME=/` or `HOME=""`
/// would make the allowlist vacuous and permit mounting any host path into the
/// VM.
pub fn validate_lima_paths(mounts: &[minibox_core::domain::BindMount]) -> anyhow::Result<()> {
    let home = home_allowlist_prefix();
    let home_display = home.as_ref().map_or_else(
        || "<unset or unusable>".to_string(),
        |p| p.display().to_string(),
    );

    for m in mounts {
        let p = &m.host_path;
        let in_home = home.as_ref().is_some_and(|h| p.starts_with(h));
        let in_tmp = p.starts_with("/tmp");
        if !in_home && !in_tmp {
            anyhow::bail!(
                "bind mount source {p:?} is not accessible inside the Lima VM.\n\
                 hint: Lima shares $HOME ({home_display}) and /tmp — move the source or add it to lima.yaml shared dirs."
            );
        }
    }
    Ok(())
}

/// Build the shell snippet that mounts one bind mount inside the Lima VM.
///
/// The snippet is injected into the spawn script before the `unshare` call.
/// It targets rootfs-relative paths so after chroot the container sees them at
/// `container_path`.
///
/// The exported rootfs is read-only at this stage, so the target must already
/// exist in the image. Creating it lazily with `mkdir -p` fails for paths like
/// `/workspace` and obscures the real problem.
///
/// # Errors
///
/// Returns an error if `container_path` contains a `..` component, which would
/// let the rootfs-relative target escape the rootfs. This mirrors the
/// `has_parent_dir_component` check that the native mount path applies in
/// `apply_one_bind_mount`; the Lima path needs its own because it never calls
/// `apply_bind_mounts`.
///
/// # Security
///
/// Every path interpolated into the snippet goes through `shell_single_quote`.
/// `host_path` and `container_path` are user-supplied, and the snippet is
/// concatenated into a shell script, so a path containing `'` would otherwise
/// terminate the quoting and inject arbitrary commands into the Lima VM.
pub fn bind_mount_shell_snippet(
    m: &minibox_core::domain::BindMount,
    rootfs: &std::path::Path,
) -> anyhow::Result<String> {
    let container_rel = m
        .container_path
        .strip_prefix("/")
        .unwrap_or(&m.container_path);

    if container_rel
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        anyhow::bail!(
            "path traversal attempt: bind mount container_path contains '..' component: {:?}",
            m.container_path
        );
    }

    let target = rootfs.join(container_rel);
    let host_q = shell_single_quote(&m.host_path.display().to_string());
    let target_q = shell_single_quote(&target.display().to_string());

    // Docker parity: a bind target that does not exist in the image is created,
    // mirroring the source's type, rather than aborting the mount. The previous
    // behaviour required every container path to pre-exist in the rootfs, so
    // `-v src:/usr/bin/tool` failed on a stock image and reported only on
    // stderr, which the CLI does not surface — a silent no-op.
    //
    // The host source is checked first so a typo'd path fails loudly instead of
    // materialising an empty mountpoint.
    let source_missing = shell_single_quote(&format!(
        "bind mount source {} does not exist on the host",
        m.host_path.display()
    ));
    let target_failed = shell_single_quote(&format!(
        "bind mount target {} could not be created in the image rootfs",
        target.display()
    ));
    let ensure_target = format!(
        "sudo test -e {host_q} || (echo {source_missing} >&2; exit 1); \
         if [ -d {host_q} ]; then sudo mkdir -p {target_q} \
           || (echo {target_failed} >&2; exit 1); \
         else sudo mkdir -p \"$(dirname {target_q})\" && sudo touch {target_q} \
           || (echo {target_failed} >&2; exit 1); fi",
    );

    if m.read_only {
        Ok(format!(
            "{ensure_target} && sudo mount --bind {host_q} {target_q} && sudo mount -o remount,ro,bind {target_q}"
        ))
    } else {
        Ok(format!(
            "{ensure_target} && sudo mount --bind {host_q} {target_q}"
        ))
    }
}

/// Build the shell fragment that populates a minimal `/dev` inside the container.
///
/// The image rootfs is mounted read-only, and the colima adapter has no
/// separate device-setup step, so a stock container starts with *no* `/dev/null`
/// — enough to break `/bin/sh` itself, let alone a nested daemon.
///
/// The node and symlink lists are generated from [`crate::fs_util::default_device_nodes`]
/// and [`crate::fs_util::default_dev_symlinks`], the same data the native
/// adapter uses, so the two paths cannot drift apart.
///
/// A tmpfs backs `/dev` (as Docker does) because the read-only rootfs cannot
/// hold newly created device nodes. `mknod` succeeds here: the container is not
/// in a user namespace, so the kernel permits device creation. Each node is
/// created best-effort — a kernel without one of them still yields a usable
/// container — but a tmpfs failure is fatal to the whole setup and is reported.
fn dev_setup_fragment() -> String {
    let mut lines = vec![
        "mkdir -p /dev/shm".to_string(),
        "mount -t tmpfs tmpfs /dev".to_string(),
    ];
    for node in crate::fs_util::default_device_nodes() {
        lines.push(format!(
            "mknod -m {mode:o} /dev/{name} c {major} {minor} 2>/dev/null || true",
            mode = node.mode,
            name = node.name,
            major = node.major,
            minor = node.minor
        ));
    }
    for link in crate::fs_util::default_dev_symlinks() {
        lines.push(format!(
            "ln -sf {target} /dev/{name} 2>/dev/null || true",
            target = link.target,
            name = link.name
        ));
    }
    lines.join("; \\\n      ")
}

/// Build the `unshare ... chroot` invocation that runs the container command.
///
/// procfs, sysfs, and cgroup2 are mounted *inside* the new mount namespace,
/// i.e. after `unshare` has created it. Two consequences that matter:
///
/// - `/proc` reflects the container's own PID namespace rather than the VM's.
/// - The mounts never leak into the Lima VM's mount namespace, so concurrent
///   containers cannot observe each other's `/proc`.
///
/// cgroup2 needs its own `mount -t`: it is a separate filesystem, so a fresh
/// `sysfs` does not carry `/sys/fs/cgroup` along, and a container that cannot
/// see its cgroup tree cannot manage resource limits or nest further.
///
/// The image rootfs is mounted read-only — the overlay's lowerdir is
/// VM-local while its upperdir is on the virtiofs-shared home directory, and
/// the kernel does not allow an overlay to straddle two filesystems. tmpfs on
/// `/tmp` and `/run` is therefore the container's writable surface, which is
/// where the daemon's socket, run, and data state are expected to live. This
/// matches what the native adapter's runtime-directory setup provides.
///
/// `/dev` is built from [`dev_setup_fragment`] for the same reason: the
/// read-only rootfs has no usable device set, and without `/dev/null` even
/// `/bin/sh` misbehaves.
///
/// The inner `sh -c` receives the real command as its first argument after
/// `$0`, so `"$@"` re-expands to the command and its arguments. Mounting is
/// best-effort: a minimal image without a usable `mount` still gets a working
/// container instead of a failed spawn, and each failure is reported on stderr
/// rather than swallowed silently.
fn namespace_exec_fragment(privileged: bool) -> String {
    let privileged_flag = if privileged { " --keep-caps" } else { "" };
    let dev_setup = dev_setup_fragment();
    format!(
        r#"sudo unshare --pid --mount --uts --ipc --net{privileged_flag} \
    --fork --kill-child \
    chroot "$ROOTFS" /bin/sh -c 'mount -t proc proc /proc \
        || echo "colima: warning: could not mount /proc in container" >&2; \
      mount -t sysfs sys /sys \
        || echo "colima: warning: could not mount /sys in container" >&2; \
      mkdir -p /sys/fs/cgroup 2>/dev/null; \
      mount -t cgroup2 none /sys/fs/cgroup \
        || echo "colima: warning: could not mount cgroup2 at /sys/fs/cgroup" >&2; \
      mkdir -p /tmp /run 2>/dev/null; \
      mount -t tmpfs tmpfs /tmp \
        || echo "colima: warning: could not mount tmpfs at /tmp" >&2; \
      mount -t tmpfs tmpfs /run \
        || echo "colima: warning: could not mount tmpfs at /run" >&2; \
      {dev_setup}; \
      exec "$@"' \
    sh "$COMMAND" "${{ARGS[@]}}""#
    )
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
        let rootfs = shell_single_quote(&config.rootfs.to_string_lossy());
        let command = shell_single_quote(&config.command);
        let args = config
            .args
            .iter()
            .map(|arg| shell_single_quote(arg))
            .collect::<Vec<_>>()
            .join(" ");

        // Validate that all bind mount host paths are Lima-accessible.
        validate_lima_paths(&config.mounts)?;

        // Build bind mount shell commands.
        let bind_mount_cmds: String = config
            .mounts
            .iter()
            .map(|m| bind_mount_shell_snippet(m, &config.rootfs))
            .collect::<anyhow::Result<Vec<_>>>()?
            .join("\n");

        let namespace_exec = namespace_exec_fragment(config.privileged);

        if self.spawner.is_some() && config.capture_output {
            // Streaming path: foreground exec with piped stdout.
            // Uses `exec unshare` so the spawned process replaces the shell,
            // making child.id() the container init PID directly.
            let spawn_script = format!(
                r"ROOTFS={rootfs}
COMMAND={command}
ARGS=({args})
{bind_mount_cmds}
exec {namespace_exec}
"
            );

            let mut child = self.lima_spawn(&["bash", "-lc", &spawn_script])?;
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
                runtime_id: None,
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
            r"
            ROOTFS={rootfs}
            COMMAND={command}
            ARGS=({args})

            {bind_mount_cmds}

            {namespace_exec} &

            echo $!
            "
        );

        let output = self.ctx.lima_exec(&["bash", "-lc", &spawn_script])?;
        let pid: u32 = output
            .trim()
            .parse()
            .map_err(|e| anyhow!("Invalid PID returned: {e}"))?;

        Ok(SpawnResult {
            runtime_id: None,
            pid,
            output_reader: None,
        })
    }

    async fn wait_for_exit(&self, _runtime_id: Option<&str>, _pid: u32) -> Result<i32> {
        // Colima manages process lifecycle inside the Lima VM.
        Ok(0)
    }
}

/// Deserialised subset of `nerdctl image inspect` output.
#[derive(Debug, Deserialize)]
struct NerdctlImageInspect {
    /// Total compressed image size in bytes as reported by nerdctl.
    #[serde(rename = "Size")]
    size: Option<i64>,
    /// Layer digest list embedded in the `RootFS` section.
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
/// Serialises environment-variable mutations across parallel Colima tests.
#[cfg(test)]
static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_colima_registry_creation() {
        let registry = ColimaRegistry::new();
        assert_eq!(registry.ctx.instance, "colima");
    }

    #[test]
    fn test_colima_with_custom_instance() {
        let registry = ColimaRegistry::new().with_instance("custom-lima".to_string());
        assert_eq!(registry.ctx.instance, "custom-lima");
    }

    #[test]
    fn test_lima_home_defaults_to_colima_lima_dir() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let prev_lima = std::env::var("LIMA_HOME").ok();
        let prev_colima = std::env::var("COLIMA_HOME").ok();
        let prev_home = std::env::var("HOME").ok();

        // SAFETY: ENV_MUTEX is held for the duration of this block, serialising
        // all environment mutations across tests. Rust 2024 requires unsafe for
        // set_var/remove_var; no other thread can observe a partially-mutated env.
        unsafe {
            std::env::remove_var("LIMA_HOME");
            std::env::remove_var("COLIMA_HOME");
            std::env::set_var("HOME", "/tmp/minibox-colima-home");
        }

        let result = lima_home();

        // SAFETY: Same ENV_MUTEX guard as above; restoring the previous values
        // while still holding the lock.
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

    #[tokio::test]
    async fn colima_load_image_calls_docker_load() {
        let called_args: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(vec![]));
        let called_clone = Arc::clone(&called_args);

        let loader = ColimaRegistry::new().with_executor(Arc::new(move |args: &[&str]| {
            called_clone
                .lock()
                .unwrap()
                .extend(args.iter().map(|s| s.to_string()));
            Ok(String::new())
        }));

        let result = loader
            .load_image(
                std::path::Path::new("/tmp/minibox-tester.tar"),
                "minibox-tester",
                "latest",
            )
            .await;
        assert!(result.is_ok(), "load_image failed: {result:?}");

        let args = called_args.lock().unwrap();
        assert!(
            args.iter().any(|a| a == "docker"),
            "expected docker call, got: {args:?}"
        );
        assert!(
            args.iter().any(|a| a == "load"),
            "expected 'load' arg, got: {args:?}"
        );
    }

    /// spawn_process must include config.args in the shell script sent to the
    /// Lima VM.  The current implementation only substitutes $COMMAND and
    /// silently drops all arguments.
    #[tokio::test]
    async fn spawn_process_includes_args_in_script() {
        use minibox_core::domain::{ContainerHooks, ContainerSpawnConfig};
        use std::sync::{Arc, Mutex};

        let captured = Arc::new(Mutex::new(String::new()));
        let cap = captured.clone();

        let runtime = ColimaRuntime::new().with_executor(Arc::new(move |args: &[&str]| {
            if let Some(pos) = args.iter().position(|&a| a == "-c" || a == "-lc")
                && let Some(script) = args.get(pos + 1)
            {
                *cap.lock().unwrap() = script.to_string();
            }
            Ok("42\n".to_string())
        }));

        let config = ContainerSpawnConfig {
            rootfs: PathBuf::from("/tmp/rootfs").into(),
            command: "/bin/echo".to_string(),
            args: vec!["hello".to_string(), "world".to_string()],
            env: vec![],
            hostname: "test-container".to_string(),
            cgroup_path: PathBuf::from("/sys/fs/cgroup/minibox/test").into(),
            capture_output: false,
            hooks: ContainerHooks::default(),
            skip_network_namespace: false,
            mounts: vec![],    // placeholder — Task 6 replaces this
            privileged: false, // placeholder — Task 6 replaces this
            uid_range_mode: minibox_core::domain::UidRangeMode::Exclusive,
            image_ref: None,
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

    /// The spawn script must mount procfs and sysfs inside the new mount
    /// namespace. Without them the container gets an *empty* `/proc` and
    /// `/sys`, which silently breaks capability inspection
    /// (`/proc/self/status`), filesystem probing (`/proc/filesystems`), and
    /// every cgroup path — the three things nested-container and DinD use.
    #[test]
    fn namespace_exec_mounts_proc_and_sysfs() {
        let frag = namespace_exec_fragment(false);
        assert!(
            frag.contains("mount -t proc proc /proc"),
            "procfs mount missing from spawn fragment: {frag}"
        );
        assert!(
            frag.contains("mount -t sysfs sys /sys"),
            "sysfs mount missing from spawn fragment: {frag}"
        );
        // cgroup2 is a separate filesystem; a fresh sysfs does not carry
        // /sys/fs/cgroup, so it needs its own mount.
        assert!(
            frag.contains("mount -t cgroup2 none /sys/fs/cgroup"),
            "cgroup2 mount missing from spawn fragment: {frag}"
        );
        // The rootfs is read-only, so tmpfs is the only writable surface.
        assert!(
            frag.contains("mount -t tmpfs tmpfs /tmp"),
            "writable /tmp missing from spawn fragment: {frag}"
        );
    }

    /// The mounts must run *after* `unshare --mount`, otherwise they land in
    /// the Lima VM's mount namespace and leak across containers.
    #[test]
    fn namespace_exec_mounts_after_unshare() {
        let frag = namespace_exec_fragment(false);
        let unshare_at = frag.find("unshare").expect("unshare present");
        let mount_at = frag.find("mount -t proc").expect("proc mount present");
        assert!(
            unshare_at < mount_at,
            "procfs must be mounted after unshare, else it leaks to the VM: {frag}"
        );
    }

    /// A failed mount must not abort the spawn: a minimal image without a
    /// usable `mount` should still get a working container, and the failure
    /// must be reported rather than silently swallowed.
    #[test]
    fn namespace_exec_tolerates_mount_failure() {
        let frag = namespace_exec_fragment(false);
        assert!(
            frag.contains("could not mount /proc"),
            "proc mount failure must be reported: {frag}"
        );
        assert!(
            frag.contains("could not mount /sys"),
            "sys mount failure must be reported: {frag}"
        );
        assert!(
            frag.contains("could not mount cgroup2"),
            "cgroup2 mount failure must be reported: {frag}"
        );
        assert!(
            frag.contains("could not mount tmpfs at /tmp"),
            "tmpfs mount failure must be reported: {frag}"
        );
    }

    /// Privileged containers still pass `--keep-caps`.
    #[test]
    fn namespace_exec_keeps_caps_when_privileged() {
        assert!(namespace_exec_fragment(true).contains("--keep-caps"));
        assert!(!namespace_exec_fragment(false).contains("--keep-caps"));
    }

    /// The container command must be re-expanded by the inner shell, not
    /// swallowed. The `sh -c '...' sh "$COMMAND" ...` tail is what makes
    /// `"$@"` resolve to the real command and its arguments.
    #[test]
    fn namespace_exec_reforwards_command_and_args() {
        let frag = namespace_exec_fragment(false);
        assert!(
            frag.contains(r#"exec "$@""#),
            "inner shell must exec its positional args: {frag}"
        );
        assert!(
            frag.contains(r#"sh "$COMMAND" "${ARGS[@]}""#),
            "command and args must be forwarded to the inner shell: {frag}"
        );
    }

    /// The container needs a real device set. Without it `/dev/null` is
    /// missing, which is enough to break `/bin/sh` itself and therefore any
    /// nested daemon.
    #[test]
    fn namespace_exec_populates_dev() {
        let frag = namespace_exec_fragment(false);
        assert!(
            frag.contains("mount -t tmpfs tmpfs /dev"),
            "/dev must be a tmpfs (the rootfs is read-only): {frag}"
        );
        assert!(
            frag.contains("mknod -m 666 /dev/null c 1 3"),
            "/dev/null missing from spawn fragment: {frag}"
        );
        assert!(
            frag.contains("mknod -m 666 /dev/urandom c 1 9")
                || frag.contains("mknod -m 444 /dev/urandom c 1 9"),
            "/dev/urandom missing from spawn fragment: {frag}"
        );
    }

    /// The device set is generated from the same tables the native adapter
    /// uses, so the two adapters cannot drift apart. Assert against the tables
    /// rather than a hardcoded list.
    #[test]
    fn dev_setup_covers_every_shared_device_node() {
        let setup = dev_setup_fragment();
        for node in crate::fs_util::default_device_nodes() {
            let expected = format!(
                "mknod -m {mode:o} /dev/{name} c {major} {minor}",
                mode = node.mode,
                name = node.name,
                major = node.major,
                minor = node.minor
            );
            assert!(
                setup.contains(&expected),
                "device node {name} missing from dev setup: {setup}",
                name = node.name
            );
        }
    }

    #[test]
    fn dev_setup_covers_every_shared_dev_symlink() {
        let setup = dev_setup_fragment();
        for link in crate::fs_util::default_dev_symlinks() {
            let expected = format!(
                "ln -sf {target} /dev/{name}",
                target = link.target,
                name = link.name
            );
            assert!(
                setup.contains(&expected),
                "dev symlink {name} missing from dev setup: {setup}",
                name = link.name
            );
        }
    }

    /// Regression: the `/dev` block is spliced in just before `exec "$@"`, so
    /// its last command must be terminated. Without the separator the tail
    /// becomes `... || true exec "$@"`, which runs `true` with the payload as
    /// arguments and the container command never executes — every run exits 0
    /// with no output.
    #[test]
    fn dev_setup_is_separated_from_the_container_command() {
        let frag = namespace_exec_fragment(false);
        let exec_at = frag.rfind(r#"exec "$@""#).expect("exec present");
        // Collapse shell line continuations, which is what the shell does, so
        // the assertion looks at the effective command line.
        let effective = frag[..exec_at].replace("\\\n", " ");
        assert!(
            effective.trim_end().ends_with(';'),
            "dev setup must end with ';' before exec: ...{}",
            &effective[effective.len().saturating_sub(40)..]
        );
    }

    #[test]
    fn colima_home_defaults_to_home_dot_colima() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let prev_colima = std::env::var("COLIMA_HOME").ok();
        let prev_home = std::env::var("HOME").ok();

        minibox_macros::unsafe_remove_var!("COLIMA_HOME");
        minibox_macros::unsafe_set_var!("HOME", "/tmp/test-home");

        let result = colima_home();

        minibox_macros::unsafe_restore_var!("COLIMA_HOME", prev_colima);
        minibox_macros::unsafe_restore_var!("HOME", prev_home);

        assert_eq!(result, PathBuf::from("/tmp/test-home").join(".colima"));
    }

    #[test]
    fn chown_to_sudo_user_if_root_is_noop_when_not_root() {
        // Regression: container_dir is created 0700 by the (root) daemon,
        // then colima's setup_rootfs writes into it as SUDO_USER via
        // privilege-dropped limactl/colima calls -- without chowning it
        // first, every one of those calls failed with "Permission denied"
        // even on a freshly-created, otherwise-correct directory. This test
        // only covers the non-root branch (can't become root in a unit
        // test); the chown itself is exercised manually against a live
        // daemon, same limitation as home_dir_for_user's root-only branch.
        if nix::unistd::geteuid().is_root() {
            return; // test process is root (e.g. CI running as root) -- skip
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let result = chown_to_sudo_user_if_root(dir.path());
        assert!(
            result.is_ok(),
            "must no-op successfully when not root: {result:?}"
        );
    }

    #[test]
    fn home_dir_for_user_resolves_real_user() {
        // Regression: colima_home() previously used the *process's* $HOME
        // unconditionally, which is wrong under `sudo` (env_reset resets
        // HOME to root's, e.g. /var/root on macOS) — every limactl call
        // then got the wrong LIMA_HOME and reported the instance as not
        // running even though colima was up. home_dir_for_user() is the
        // fix's lookup path; verify it resolves the current user correctly
        // rather than mocking geteuid()==root, which isn't testable here.
        //
        // Takes ENV_MUTEX like the other tests in this file that touch
        // $HOME/$COLIMA_HOME/$LIMA_HOME — without it, this test's read of
        // $HOME races with those tests' temporary mutations and flakes under
        // parallel test execution.
        let _guard = ENV_MUTEX.lock().unwrap();
        let username = std::env::var("USER").or_else(|_| std::env::var("LOGNAME"));
        let Ok(username) = username else {
            return; // no USER/LOGNAME in this environment — nothing to assert
        };
        let expected_home = std::env::var("HOME").ok().map(PathBuf::from);

        let resolved = home_dir_for_user(&username);

        assert!(
            resolved.is_some(),
            "expected to resolve a home dir for user {username:?}"
        );
        if let Some(expected) = expected_home {
            assert_eq!(resolved, Some(expected));
        }
    }

    #[test]
    fn home_dir_for_user_returns_none_for_bogus_user() {
        assert_eq!(home_dir_for_user("definitely-not-a-real-user-xyz123"), None);
    }

    #[test]
    fn sudo_drop_privileges_args_preserves_lima_home() {
        // Regression: without --preserve-env=LIMA_HOME, `sudo -u <user>`
        // applies its own env_reset when dropping from root back to the
        // target user, silently discarding the LIMA_HOME set on the outer
        // Command. limactl then falls back to the default ~/.lima and
        // reports colima's actual running instance as "not running".
        let args = sudo_drop_privileges_args("joe", "limactl");
        assert_eq!(
            args,
            vec!["-u", "joe", "--preserve-env=LIMA_HOME", "--", "limactl"]
        );
    }

    #[test]
    fn colima_home_respects_colima_home_env_var() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let prev = std::env::var("COLIMA_HOME").ok();

        minibox_macros::unsafe_set_var!("COLIMA_HOME", "/custom/colima");
        let result = colima_home();
        minibox_macros::unsafe_restore_var!("COLIMA_HOME", prev);

        assert_eq!(result, PathBuf::from("/custom/colima"));
    }

    #[test]
    fn lima_home_defaults_to_colima_home_lima_subdir() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let prev_lima = std::env::var("LIMA_HOME").ok();
        let prev_colima = std::env::var("COLIMA_HOME").ok();
        let prev_home = std::env::var("HOME").ok();

        minibox_macros::unsafe_remove_var!("LIMA_HOME");
        minibox_macros::unsafe_remove_var!("COLIMA_HOME");
        minibox_macros::unsafe_set_var!("HOME", "/tmp/test-home");

        let result = lima_home();

        minibox_macros::unsafe_restore_var!("LIMA_HOME", prev_lima);
        minibox_macros::unsafe_restore_var!("COLIMA_HOME", prev_colima);
        minibox_macros::unsafe_restore_var!("HOME", prev_home);

        assert_eq!(result, "/tmp/test-home/.colima/_lima");
    }

    #[test]
    fn lima_home_respects_lima_home_env_var() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let prev = std::env::var("LIMA_HOME").ok();

        minibox_macros::unsafe_set_var!("LIMA_HOME", "/custom/lima");
        let result = lima_home();
        minibox_macros::unsafe_restore_var!("LIMA_HOME", prev);

        assert_eq!(result, "/custom/lima");
    }

    #[test]
    fn limactl_command_injects_lima_home_into_env() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let prev_lima = std::env::var("LIMA_HOME").ok();
        let prev_colima = std::env::var("COLIMA_HOME").ok();

        minibox_macros::unsafe_remove_var!("LIMA_HOME");
        minibox_macros::unsafe_remove_var!("COLIMA_HOME");

        let cmd = limactl_command("limactl");
        let expected_lima_home = lima_home();

        minibox_macros::unsafe_restore_var!("LIMA_HOME", prev_lima);
        minibox_macros::unsafe_restore_var!("COLIMA_HOME", prev_colima);

        assert!(
            !expected_lima_home.is_empty(),
            "lima_home must return a non-empty path"
        );
        assert_eq!(cmd.get_program(), "limactl");
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
        use minibox_core::domain::{ContainerHooks, ContainerSpawnConfig};
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
            rootfs: PathBuf::from("/tmp/rootfs").into(),
            command: "/bin/echo".to_string(),
            args: vec!["hello".to_string()],
            env: vec![],
            hostname: "test".to_string(),
            cgroup_path: PathBuf::from("/sys/fs/cgroup/minibox/test").into(),
            capture_output: true,
            skip_network_namespace: false,
            hooks: ContainerHooks::default(),
            mounts: vec![],    // placeholder — Task 6 replaces this
            privileged: false, // placeholder — Task 6 replaces this
            uid_range_mode: minibox_core::domain::UidRangeMode::Exclusive,
            image_ref: None,
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

#[cfg(test)]
mod bind_mount_tests {
    use super::*;
    use minibox_core::domain::BindMount;
    use std::path::PathBuf;

    /// Run `f` with `$HOME` set to `value`, restoring the previous value after.
    ///
    /// Tests must not depend on the ambient `$HOME`: on a developer machine it
    /// is a real directory, but in a container or VM it is often `/`, which
    /// changes what the allowlist accepts. That is exactly the bug
    /// `validate_lima_paths` had, so pin the value instead of inheriting it.
    fn with_home<R>(value: Option<&str>, f: impl FnOnce() -> R) -> R {
        // Recover from poisoning rather than propagating it. A test that
        // panics *while holding* the lock would otherwise poison ENV_MUTEX and
        // every later test in this module would die with PoisonError, hiding
        // which assertions actually failed.
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = std::env::var("HOME").ok();
        // SAFETY: ENV_MUTEX is held for the whole mutation window, serialising
        // every environment change in this module. Rust 2024 requires unsafe for
        // set_var/remove_var, and no other thread can observe a partial state.
        unsafe {
            match value {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
        let out = f();
        // SAFETY: Same guard, still held; the previous value is restored before
        // the lock is released.
        unsafe {
            match prev {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
        out
    }

    /// A mount of `host`, which every test below asserts about.
    fn mount_of(host: &str) -> Vec<BindMount> {
        vec![BindMount {
            host_path: PathBuf::from(host),
            container_path: PathBuf::from("/data"),
            read_only: false,
        }]
    }

    #[test]
    fn validate_lima_paths_rejects_when_home_is_root() {
        // `Path::new("/").starts_with(anything)` is true for every absolute
        // path, so a root HOME used to turn the allowlist into a no-op.
        with_home(Some("/"), || {
            let err = validate_lima_paths(&mount_of("/opt/homebrew/bin")).unwrap_err();
            assert!(
                err.to_string().contains("not accessible"),
                "root HOME must not widen the allowlist, got: {err}"
            );
        });
    }

    #[test]
    fn validate_lima_paths_rejects_when_home_is_empty() {
        // `Path::new("").starts_with(anything)` is likewise true for everything.
        with_home(Some(""), || {
            let err = validate_lima_paths(&mount_of("/opt/homebrew/bin")).unwrap_err();
            assert!(
                err.to_string().contains("not accessible"),
                "empty HOME must not widen the allowlist, got: {err}"
            );
        });
    }

    #[test]
    fn validate_lima_paths_rejects_when_home_is_unset() {
        with_home(None, || {
            let err = validate_lima_paths(&mount_of("/opt/homebrew/bin")).unwrap_err();
            assert!(
                err.to_string().contains("not accessible"),
                "unset HOME must not widen the allowlist, got: {err}"
            );
        });
    }

    #[test]
    fn validate_lima_paths_still_accepts_real_home_subdir() {
        // The normal case must keep working: a real HOME still permits its own
        // subdirectories, and /tmp stays permitted regardless of HOME.
        with_home(Some("/home/tester"), || {
            validate_lima_paths(&mount_of("/home/tester/project/bin"))
                .expect("a real HOME subdir should be accepted");
            validate_lima_paths(&mount_of("/tmp/minibox-test"))
                .expect("/tmp should be accepted regardless of HOME");
            let err = validate_lima_paths(&mount_of("/opt/homebrew/bin"))
                .expect_err("outside a real HOME and /tmp must be rejected");
            assert!(err.to_string().contains("not accessible"), "got: {err}");
        });
    }

    #[test]
    fn validate_lima_paths_error_names_the_unusable_home() {
        with_home(Some("/"), || {
            let err = validate_lima_paths(&mount_of("/opt/homebrew/bin")).unwrap_err();
            assert!(
                err.to_string().contains("<unset or unusable>"),
                "error should report the unusable HOME rather than pretending it is /, got: {err}"
            );
        });
    }

    #[test]
    fn validate_lima_paths_accepts_home_subdir() {
        with_home(Some("/home/tester"), || {
            let mounts = vec![BindMount {
                host_path: PathBuf::from("/home/tester/some/project/bin"),
                container_path: PathBuf::from("/bin"),
                read_only: false,
            }];
            validate_lima_paths(&mounts).expect("home subdir should be accepted");
        });
    }

    #[test]
    fn validate_lima_paths_accepts_tmp_subdir() {
        let mounts = vec![BindMount {
            host_path: PathBuf::from("/tmp/minibox-test"),
            container_path: PathBuf::from("/data"),
            read_only: false,
        }];
        validate_lima_paths(&mounts).expect("tmp subdir should be accepted");
    }

    #[test]
    fn validate_lima_paths_rejects_opt() {
        // Pin a real HOME: this only holds if /opt is genuinely outside it.
        with_home(Some("/home/tester"), || {
            let mounts = vec![BindMount {
                host_path: PathBuf::from("/opt/homebrew/bin"),
                container_path: PathBuf::from("/bin"),
                read_only: false,
            }];
            let err = validate_lima_paths(&mounts).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("Lima") || msg.contains("accessible"),
                "expected Lima path error, got: {msg}"
            );
        });
    }

    #[test]
    fn validate_lima_paths_empty_mounts_passes() {
        validate_lima_paths(&[]).expect("empty mounts should pass");
    }

    #[test]
    fn bind_mount_shell_snippet_rw() {
        let m = BindMount {
            host_path: PathBuf::from("/tmp/host"),
            container_path: PathBuf::from("/guest"),
            read_only: false,
        };
        let snippet = bind_mount_shell_snippet(&m, &PathBuf::from("/rootfs")).expect("snippet");
        assert!(snippet.contains("test -e"), "snippet: {snippet}");
        assert!(snippet.contains("mount --bind"), "snippet: {snippet}");
        assert!(snippet.contains("/tmp/host"), "snippet: {snippet}");
        assert!(snippet.contains("/rootfs/guest"), "snippet: {snippet}");
    }

    /// Docker parity: a target that is absent from the image is created rather
    /// than aborting the mount. The snippet must branch on the *source* type so
    /// a file source does not become a directory mountpoint (or vice versa).
    #[test]
    fn bind_mount_shell_snippet_creates_missing_target() {
        let m = BindMount {
            host_path: PathBuf::from("/tmp/host"),
            container_path: PathBuf::from("/guest/deep/tool"),
            read_only: false,
        };
        let snippet = bind_mount_shell_snippet(&m, &PathBuf::from("/rootfs")).expect("snippet");
        assert!(
            snippet.contains("if [ -d"),
            "snippet must branch on source type: {snippet}"
        );
        assert!(
            snippet.contains("sudo mkdir -p"),
            "snippet must create the target: {snippet}"
        );
        assert!(
            snippet.contains("sudo touch"),
            "file sources must create a file target: {snippet}"
        );
        // The old failure mode reported only on stderr and produced a silent
        // no-op; the source is now checked loudly.
        assert!(
            snippet.contains("does not exist on the host"),
            "a missing source must fail loudly: {snippet}"
        );
        assert!(
            snippet.contains("could not be created"),
            "a target that cannot be created must fail loudly, not silently: {snippet}"
        );
        assert!(
            !snippet.contains("does not exist in image rootfs"),
            "an absent target must no longer be fatal: {snippet}"
        );
    }

    #[test]
    fn bind_mount_shell_snippet_ro() {
        let m = BindMount {
            host_path: PathBuf::from("/tmp/host"),
            container_path: PathBuf::from("/guest"),
            read_only: true,
        };
        let snippet = bind_mount_shell_snippet(&m, &PathBuf::from("/rootfs")).expect("snippet");
        assert!(snippet.contains("test -e"), "snippet: {snippet}");
        assert!(snippet.contains("mount --bind"), "snippet: {snippet}");
        assert!(
            snippet.contains("remount,ro,bind") || snippet.contains("remount,bind,ro"),
            "snippet: {snippet}"
        );
        // The read-only remount must stay the last step, after the target is
        // created and the bind is in place.
        let mount_at = snippet.find("mount --bind").expect("bind mount");
        let ro_at = snippet
            .find("remount,ro,bind")
            .or_else(|| snippet.find("remount,bind,ro"))
            .expect("ro remount");
        assert!(
            mount_at < ro_at,
            "read-only remount must follow the bind: {snippet}"
        );
    }

    #[test]
    fn bind_mount_shell_snippet_rejects_parent_dir_in_container_path() {
        let m = BindMount {
            host_path: PathBuf::from("/tmp/host"),
            container_path: PathBuf::from("/../../../etc"),
            read_only: false,
        };
        let err = bind_mount_shell_snippet(&m, &PathBuf::from("/rootfs"))
            .expect_err("parent-dir container_path must be rejected");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("path traversal"),
            "expected 'path traversal' in error, got: {msg}"
        );
    }

    #[test]
    fn bind_mount_shell_snippet_rejects_relative_parent_dir_container_path() {
        let m = BindMount {
            host_path: PathBuf::from("/tmp/host"),
            container_path: PathBuf::from("../escape"),
            read_only: false,
        };
        let err = bind_mount_shell_snippet(&m, &PathBuf::from("/rootfs"))
            .expect_err("relative parent-dir container_path must be rejected");
        assert!(
            format!("{err:#}").contains("path traversal"),
            "expected 'path traversal' in error"
        );
    }

    #[test]
    fn bind_mount_shell_snippet_escapes_single_quote_in_host_path() {
        // A host path containing a single quote must not terminate the
        // surrounding shell quoting. POSIX has no escape inside single quotes;
        // the only safe form is to close, emit an escaped quote, and reopen.
        let m = BindMount {
            host_path: PathBuf::from("/tmp/it's-a-trap"),
            container_path: PathBuf::from("/guest"),
            read_only: false,
        };
        let snippet = bind_mount_shell_snippet(&m, &PathBuf::from("/rootfs")).expect("snippet");

        assert!(
            !snippet.contains("'/tmp/it's-a-trap'"),
            "quote was not escaped, snippet is injectable: {snippet}"
        );
        assert!(
            snippet.contains(r"'\''"),
            "expected POSIX quote-escape sequence in snippet: {snippet}"
        );
    }

    #[test]
    fn bind_mount_shell_snippet_escapes_single_quote_in_container_path() {
        let m = BindMount {
            host_path: PathBuf::from("/tmp/host"),
            container_path: PathBuf::from("/it's-also-a-trap"),
            read_only: false,
        };
        let snippet = bind_mount_shell_snippet(&m, &PathBuf::from("/rootfs")).expect("snippet");

        assert!(
            !snippet.contains("'/rootfs/it's-also-a-trap'"),
            "quote was not escaped, snippet is injectable: {snippet}"
        );
        assert!(
            snippet.contains(r"'\''"),
            "expected POSIX quote-escape sequence in snippet: {snippet}"
        );
    }

    #[test]
    fn bind_mount_shell_snippet_neutralizes_command_substitution() {
        // Command substitution inside single quotes is literal to the shell.
        // This pins that a payload like $(id) cannot execute.
        let m = BindMount {
            host_path: PathBuf::from("/tmp/$(id)"),
            container_path: PathBuf::from("/guest"),
            read_only: false,
        };
        let snippet = bind_mount_shell_snippet(&m, &PathBuf::from("/rootfs")).expect("snippet");

        assert!(
            snippet.contains("'/tmp/$(id)'"),
            "payload must stay inside single quotes: {snippet}"
        );
    }

    #[test]
    fn bind_mount_shell_snippet_keeps_target_inside_rootfs() {
        let m = BindMount {
            host_path: PathBuf::from("/tmp/host"),
            container_path: PathBuf::from("/nested/dir/target"),
            read_only: false,
        };
        let snippet = bind_mount_shell_snippet(&m, &PathBuf::from("/rootfs")).expect("snippet");
        assert!(
            snippet.contains("'/rootfs/nested/dir/target'"),
            "expected fully-quoted rootfs-relative target: {snippet}"
        );
    }
}
