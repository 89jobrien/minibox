//! `SmolVM` adapter suite — lightweight Linux VMs via smolmachines.
//!
//! Delegates container operations into a smolmachines VM. smolmachines
//! uses libkrun (a lightweight VMM) to boot Linux VMs with sub-second
//! cold starts. Works on both macOS (Apple Silicon / Intel) and Linux.
//!
//! Selected by `MINIBOX_ADAPTER=smolvm`. Compiled on all platforms.
//!
//! Requirements:
//! - smolmachines installed (<https://smolmachines.com>)
//!   - macOS: `brew install smolvm`
//!   - Linux: see smolmachines docs

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use minibox_core::adapt;
use minibox_core::domain::{
    ContainerRuntime, ContainerSpawnConfig, ImageLoader, ImageMetadata, ImageRegistry,
    ResourceConfig, ResourceLimiter, RootfsLayout, RuntimeCapabilities, SpawnResult,
};
use minibox_core::image::ImageStore;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use crate::adapters::DockerHubRegistry;

/// Default smolvm image used for container operations.
const DEFAULT_IMAGE: &str = "ubuntu:24.04";
/// Host tarball directory mount point used while importing local images.
const LOAD_MOUNT: &str = "/mnt/minibox-load";
/// Timeout for local image imports into the VM.
const LOAD_TIMEOUT_SECS: u32 = 600;
/// POSIX shell script that imports a tarball and retags whatever docker loaded.
///
/// The stock `ubuntu:24.04` guest image ships no container runtime, so this
/// installs `docker.io` on first use (subsequent `smolvm machine run` calls
/// that reuse the same cached VM see `docker` already on PATH and skip
/// straight past it). Installing the package does not start `dockerd` on its
/// own in this guest image (no systemd unit gets brought up on a one-off
/// `smolvm machine run` invocation) — the daemon is started explicitly and
/// polled until its socket answers before `docker load` runs.
const DOCKER_LOAD_AND_TAG_SCRIPT: &str = r#"set -eu
tarball="$1"
target="$2"
if ! command -v docker >/dev/null 2>&1; then
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq && apt-get install -y -qq docker.io >/dev/null
fi
if ! docker info >/dev/null 2>&1; then
    dockerd >/var/log/dockerd.log 2>&1 &
    for _ in $(seq 1 30); do
        docker info >/dev/null 2>&1 && break
        sleep 1
    done
fi
out="$(docker load -i "$tarball")"
printf '%s\n' "$out"
loaded="$(printf '%s\n' "$out" | sed -n 's/^Loaded image: //p' | tail -n 1)"
if [ -n "$loaded" ]; then
    docker tag "$loaded" "$target"
    exit 0
fi
loaded_id="$(printf '%s\n' "$out" | sed -n 's/^Loaded image ID: //p' | tail -n 1)"
if [ -n "$loaded_id" ]; then
    docker tag "$loaded_id" "$target"
    exit 0
fi
printf '%s\n' "docker load did not report a loaded image or image ID" >&2
exit 1
"#;

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

/// Output from a synchronous smolvm command execution.
struct SmolVmOutput {
    stdout: String,
    exit_code: i32,
}

/// Run a command via the real `smolvm` binary with volume mounts and env vars.
///
/// Returns both stdout and exit code. Does NOT treat non-zero exit as an error
/// — the caller (handler) decides how to handle the exit code.
fn smolvm_exec_full(
    image: &str,
    args: &[&str],
    volumes: &[(&str, &str)],
    env: &[(&str, &str)],
    timeout_secs: u32,
) -> Result<SmolVmOutput> {
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

    let mut combined = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.stderr.is_empty() {
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
    }

    Ok(SmolVmOutput {
        stdout: combined,
        exit_code: output.status.code().unwrap_or(1),
    })
}

/// Run a command inside the smolvm VM and return its stdout.
///
/// The real `smolvm` invocation blocks on a synchronous `Command::output()`
/// call that can take well over a minute (VM boot + image pull), so it runs
/// on a blocking-pool thread via `spawn_blocking` rather than inline on the
/// async runtime — otherwise it starves whichever tokio worker picked up
/// this request's task.
async fn run_vm_exec(
    image: &str,
    executor: Option<&SmolVmExecutor>,
    args: &[&str],
) -> Result<String> {
    if let Some(exec) = executor {
        return exec(args);
    }
    let image = image.to_owned();
    let args_owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    tokio::task::spawn_blocking(move || {
        let arg_refs: Vec<&str> = args_owned.iter().map(String::as_str).collect();
        smolvm_exec(&image, &arg_refs)
    })
    .await
    .map_err(|e| anyhow!("smolvm_exec: join error: {e}"))?
}

// ============================================================================
// SmolVm Image Registry Adapter
// ============================================================================

/// `SmolVM` implementation of [`ImageRegistry`].
///
/// Pulls images on the host via [`DockerHubRegistry`] (real OCI registry
/// client, no VM/network round-trip through the guest) to populate the
/// shared [`ImageStore`] for `has_image`/`mbx images`/size-limit checks. The
/// actual container run boots the requested image directly via smolvm's own
/// native OCI pull (see [`SmolVmRuntime::spawn_process`]) — no docker
/// dependency anywhere in this path.
pub struct SmolVmRegistry {
    /// Image to use for the VM (default: ubuntu:24.04).
    image: String,
    /// Optional injected executor used in tests to avoid real smolvm calls.
    executor: Option<SmolVmExecutor>,
    /// Host-side OCI registry client backing the actual pull.
    inner: DockerHubRegistry,
}

impl SmolVmRegistry {
    /// Create a new registry adapter using the default smolvm image and the
    /// given host-side image store (typically the daemon's shared
    /// `state.image_store`, so pulls are cached across adapters).
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying OCI registry HTTP client cannot be
    /// initialised (e.g. TLS initialisation failure).
    pub fn new(store: Arc<ImageStore>) -> Result<Self> {
        let inner = DockerHubRegistry::new(store)?;
        Ok(Self {
            image: DEFAULT_IMAGE.to_string(),
            executor: None,
            inner,
        })
    }

    /// Override the smolvm VM image (default: `ubuntu:24.04`).
    #[must_use]
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
    async fn vm_exec(&self, args: &[&str]) -> Result<String> {
        run_vm_exec(&self.image, self.executor.as_ref(), args).await
    }

    /// Build the image reference used for VM-local docker operations.
    ///
    /// Strips the `library/` namespace prefix so tags written by [`ImageLoader::load_image`]
    /// line up with the names [`ImageRegistry::has_image`]/[`ImageRegistry::pull_image`] look
    /// for. Docker itself normalizes pulled official images this way (`docker pull
    /// library/alpine` is locally tagged `alpine:latest`, not `library/alpine:latest`); `docker
    /// tag` does not apply the same normalization, so `load_image` must strip the prefix
    /// itself to stay consistent — otherwise a locally loaded image is tagged
    /// `library/foo:latest` while `has_image`/`pull_image` look for `foo:latest`, and `mbx run`
    /// reports the image missing and attempts a real network pull.
    fn target_ref(name: &str, tag: &str) -> String {
        let short_name = name.strip_prefix("library/").unwrap_or(name);
        format!("{short_name}:{tag}")
    }

    /// Return the canonical host tarball path, parent directory, and VM guest path.
    fn load_paths(path: &Path) -> Result<(PathBuf, PathBuf, String)> {
        if !path.exists() {
            bail!("image tarball not found: {}", path.display());
        }
        let tarball = path
            .canonicalize()
            .with_context(|| format!("canonicalize image tarball {}", path.display()))?;
        let parent = tarball
            .parent()
            .context("image tarball has no parent directory")?
            .to_path_buf();
        let file_name = tarball
            .file_name()
            .and_then(|name| name.to_str())
            .context("image tarball filename is not valid UTF-8")?;
        let guest_path = format!("{LOAD_MOUNT}/{file_name}");
        Ok((tarball, parent, guest_path))
    }
}

#[async_trait]
impl ImageRegistry for SmolVmRegistry {
    /// Check if an image has already been pulled to the host-side image
    /// store. Unlike the VM-local docker cache (which is discarded whenever
    /// the ephemeral guest VM exits), the host store persists across runs,
    /// so this is both faster and more accurate than asking the guest.
    async fn has_image(&self, name: &str, tag: &str) -> bool {
        self.inner.has_image(name, tag).await
    }

    /// Pull an image on the host via [`DockerHubRegistry`] to populate the
    /// shared [`ImageStore`] (used by `has_image`, `mbx images`, and
    /// size-limit enforcement).
    ///
    /// Unlike the previous design, this no longer packages the pulled layers
    /// into a docker-load tarball or imports them into a VM-local docker
    /// cache: `SmolVmRuntime::spawn_process` passes the requested image ref
    /// straight to `smolvm machine run --image`, which pulls and boots the
    /// OCI image natively via smolvm's own registry client — no docker
    /// binary needed on the host or inside the guest.
    async fn pull_image(
        &self,
        image_ref: &crate::image::reference::ImageRef,
    ) -> Result<ImageMetadata> {
        self.inner.pull_image(image_ref).await
    }

    /// Layer paths live inside the VM's image cache. Return a stable VM-local
    /// marker so the shared run pipeline can proceed; `SmolVmFilesystem`
    /// treats this as metadata and does not perform host overlay setup.
    fn get_image_layers(&self, name: &str, tag: &str) -> Result<Vec<PathBuf>> {
        Ok(vec![PathBuf::from(format!("smolvm-image/{name}:{tag}"))])
    }
}

#[async_trait]
impl ImageLoader for SmolVmRegistry {
    /// Load a local image tarball into the smolvm VM-local Docker image cache.
    ///
    /// The tarball directory is mounted into the VM, `docker load` imports the
    /// image, and the loaded image/image-id is tagged as `name:tag` so the same
    /// smolvm registry cache that `mbx run` checks can find it later.
    async fn load_image(&self, path: &Path, name: &str, tag: &str) -> Result<()> {
        let target = Self::target_ref(name, tag);
        let (tarball, parent, guest_path) = Self::load_paths(path)?;

        // Write the loader script to a file inside the mounted directory
        // rather than passing it inline as a `sh -c <script>` argument.
        // smolvm's `machine run -- <args>` transport does not preserve a
        // multi-line string as a single argv element (embedded newlines get
        // re-split), which corrupts `-c`'s script argument and causes the
        // guest to attempt to exec a bare word from mid-script (observed:
        // `docker` — the first external command name in the script) instead
        // of running it. A script file survives the transport intact.
        let script_path = parent.join("docker-load-and-tag.sh");
        std::fs::write(&script_path, DOCKER_LOAD_AND_TAG_SCRIPT)
            .with_context(|| format!("write loader script to {}", script_path.display()))?;
        let guest_script_path = format!("{LOAD_MOUNT}/docker-load-and-tag.sh");

        if self.executor.is_some() {
            self.vm_exec(&[
                "sh",
                &guest_script_path,
                tarball
                    .to_str()
                    .context("image tarball path is not valid UTF-8")?,
                &target,
            ])
            .await?;
            return Ok(());
        }

        let parent_str = parent
            .to_str()
            .context("image tarball parent path is not valid UTF-8")?;
        let image = self.image.clone();
        let parent = parent_str.to_owned();
        let target_for_exec = target.clone();
        let output = tokio::task::spawn_blocking(move || {
            smolvm_exec_full(
                &image,
                &["sh", &guest_script_path, &guest_path, &target_for_exec],
                &[(parent.as_str(), LOAD_MOUNT)],
                &[],
                LOAD_TIMEOUT_SECS,
            )
        })
        .await
        .map_err(|e| anyhow!("smolvm docker load: join error: {e}"))??;

        if output.exit_code != 0 {
            bail!(
                "smolvm docker load failed for {} as {target}: {}",
                path.display(),
                output.stdout
            );
        }

        Ok(())
    }
}

// ============================================================================
// SmolVm Container Runtime Adapter
// ============================================================================

/// `SmolVM` implementation of [`ContainerRuntime`].
///
/// Spawns container processes by running commands inside a smolvm VM via
/// `smolvm machine run`. Each `spawn_process` call boots a fresh VM instance.
pub struct SmolVmRuntime {
    /// Image to use for the VM.
    image: String,
    /// Optional injected executor used in tests.
    executor: Option<SmolVmExecutor>,
    /// Exit code from the last synchronous smolvm execution.
    /// smolvm runs commands synchronously in `spawn_process`, so
    /// the exit code is available immediately rather than via waitpid.
    last_exit_code: std::sync::Mutex<i32>,
}

impl SmolVmRuntime {
    /// Create a new runtime adapter using the default smolvm image.
    #[must_use]
    pub fn new() -> Self {
        Self {
            image: DEFAULT_IMAGE.to_string(),
            executor: None,
            last_exit_code: std::sync::Mutex::new(0),
        }
    }

    /// Inject a custom executor for testing.
    pub fn with_executor(mut self, executor: SmolVmExecutor) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Run a command inside the smolvm VM and return its stdout.
    ///
    /// See [`SmolVmRegistry::vm_exec`] — same rationale for `spawn_blocking`.
    async fn vm_exec(&self, args: &[&str]) -> Result<String> {
        run_vm_exec(&self.image, self.executor.as_ref(), args).await
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
    // qual:allow(complexity) reason: "smolvm CLI invocation with mount/env assembly"
    async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<SpawnResult> {
        let mut command = vec![config.command.as_str()];
        let args: Vec<&str> = config
            .args
            .iter()
            .map(std::string::String::as_str)
            .collect();
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

        let (stdout, exit_code) = if self.executor.is_some() {
            // Use the test executor — flatten command into a single arg list.
            (self.vm_exec(&command).await?, 0)
        } else {
            const DEFAULT_EXEC_TIMEOUT_SECS: u32 = 600;
            // Boot the actual requested container image (e.g. "alpine",
            // "python:3.12-alpine") directly via smolvm's native OCI pull,
            // not the fixed VM base image — `config.image_ref` is set by
            // `handle_run` for every request. Falls back to `self.image`
            // only for callers that don't set it (there should be none in
            // the smolvm suite; kept defensive rather than panicking).
            let image = config
                .image_ref
                .clone()
                .unwrap_or_else(|| self.image.clone());
            let command_owned: Vec<String> = command.iter().map(|s| (*s).to_owned()).collect();
            let volumes_owned = volumes.clone();
            let env_pairs_owned = env_pairs.clone();
            let result = tokio::task::spawn_blocking(move || {
                let command_refs: Vec<&str> = command_owned.iter().map(String::as_str).collect();
                let vol_refs: Vec<(&str, &str)> = volumes_owned
                    .iter()
                    .map(|(h, g)| (h.as_str(), g.as_str()))
                    .collect();
                let env_refs: Vec<(&str, &str)> = env_pairs_owned
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                smolvm_exec_full(
                    &image,
                    &command_refs,
                    &vol_refs,
                    &env_refs,
                    DEFAULT_EXEC_TIMEOUT_SECS,
                )
            })
            .await
            .map_err(|e| anyhow!("smolvm_exec_full: join error: {e}"))??;
            (result.stdout, result.exit_code)
        };

        // Store exit code for wait_for_exit.
        // Poisoned mutex is unrecoverable — the process that held the lock panicked.
        {
            #[allow(clippy::expect_used)]
            let mut guard = self.last_exit_code.lock().expect("lock poisoned");
            *guard = exit_code;
        }

        // The command already ran synchronously. Pipe captured output into
        // an OwnedFd so the handler's streaming loop can read it.
        #[cfg(unix)]
        let output_reader = {
            let (read_fd, write_fd) =
                nix::unistd::pipe().map_err(|e| anyhow!("pipe() failed: {e}"))?;
            let stdout_bytes = stdout.as_bytes();
            let _ = nix::unistd::write(&write_fd, stdout_bytes);
            drop(write_fd);
            Some(read_fd)
        };
        #[cfg(not(unix))]
        let output_reader = None;

        Ok(SpawnResult {
            runtime_id: None,
            pid: 0,
            output_reader,
        })
    }

    async fn wait_for_exit(&self, _runtime_id: Option<&str>, _pid: u32) -> Result<i32> {
        // The command already ran synchronously in spawn_process.
        // Return the stored exit code.
        #[allow(clippy::expect_used)]
        let code = *self.last_exit_code.lock().expect("lock poisoned");
        Ok(code)
    }
}

// ============================================================================
// SmolVm Filesystem Adapter
// ============================================================================

/// `SmolVM` implementation of [`crate::domain::FilesystemProvider`].
///
/// Filesystem operations are handled inside the VM. All methods are no-ops
/// on the host side — the VM's kernel manages overlay mounts and `pivot_root`.
pub struct SmolVmFilesystem;

impl SmolVmFilesystem {
    /// Create a new filesystem adapter.
    #[must_use]
    pub const fn new() -> Self {
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
            merged_dir: container_dir.to_path_buf().into(),
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
    /// `pivot_root` runs inside the VM, not on the host.
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

/// `SmolVM` implementation of [`ResourceLimiter`].
///
/// Cgroup operations are handled inside the VM's Linux kernel. All methods
/// are no-ops on the host side.
pub struct SmolVmLimiter;

impl SmolVmLimiter {
    /// Create a new resource limiter adapter.
    #[must_use]
    pub const fn new() -> Self {
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

// SmolVmRegistry's `new()` takes an `Arc<ImageStore>` and returns `Result`,
// so it doesn't fit `adapt!`'s no-arg-`Default`-constructor contract — just
// give it `AsAny`. The other three adapters still take no arguments.
minibox_core::as_any!(SmolVmRegistry);
adapt!(SmolVmFilesystem, SmolVmLimiter, SmolVmRuntime);

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

    /// Build a registry backed by a throwaway temp-dir image store, for tests
    /// that don't care about the specific store location.
    fn test_registry() -> SmolVmRegistry {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(ImageStore::new(tmp.path().join("images")).expect("ImageStore::new"));
        std::mem::forget(tmp); // keep tempdir alive for the registry's lifetime
        SmolVmRegistry::new(store).expect("SmolVmRegistry::new")
    }

    /// Compile-time and runtime check: all four adapters satisfy the required domain traits
    /// and can be instantiated.
    #[test]
    fn adapter_implements_all_traits() {
        // Compile-time trait satisfaction (fails to compile if a trait is not implemented).
        let _ = _assert_image_registry::<SmolVmRegistry>;
        let _ = _assert_container_runtime::<SmolVmRuntime>;
        let _ = _assert_filesystem_provider::<SmolVmFilesystem>;
        let _ = _assert_resource_limiter::<SmolVmLimiter>;
        // Runtime: verify the adapters can be constructed.
        let registry = test_registry();
        let runtime = SmolVmRuntime::new();
        let filesystem = SmolVmFilesystem::new();
        let limiter = SmolVmLimiter::new();
        assert!(
            !registry.image.is_empty(),
            "SmolVmRegistry must have a non-empty default image"
        );
        drop((runtime, filesystem, limiter));
    }

    /// `has_image` now checks the host-side image store (persists across
    /// ephemeral VM instances) instead of asking the guest, so it reports
    /// `true` once `inner`'s backing store has the image cached — with no
    /// VM executor involved at all.
    #[tokio::test]
    async fn registry_has_image_checks_host_store() {
        let registry = test_registry();
        assert!(!registry.has_image("alpine", "latest").await);
    }

    /// `target_ref` strips the `library/` namespace prefix so a locally loaded image's
    /// docker tag matches what `has_image`/`pull_image` look for (issue #457 regression:
    /// without this, `mbx load --name library/foo` tagged `library/foo:latest` but `mbx
    /// run foo` checked for `foo:latest` and never found it).
    #[test]
    fn target_ref_strips_library_prefix() {
        assert_eq!(
            SmolVmRegistry::target_ref("library/foo", "latest"),
            "foo:latest"
        );
        assert_eq!(SmolVmRegistry::target_ref("foo", "latest"), "foo:latest");
        assert_eq!(
            SmolVmRegistry::target_ref("ghcr.io/org/image", "v1"),
            "ghcr.io/org/image:v1"
        );
    }

    /// Filesystem setup_rootfs returns the container_dir as merged_dir.
    #[test]
    fn filesystem_setup_rootfs_returns_placeholder() {
        let fs = SmolVmFilesystem::new();
        let dir = PathBuf::from("/tmp/test-container");
        let layout = fs.setup_rootfs(&[], &dir).expect("setup_rootfs");
        assert_eq!(&*layout.merged_dir, dir.as_path());
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
