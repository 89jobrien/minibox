//! Request handlers for each daemon operation.
//!
//! Each public function corresponds to one `DaemonRequest` variant and
//! returns a `DaemonResponse`.  Errors are caught and returned as
//! `DaemonResponse::Error` so the daemon never panics on bad input.
//!
//! # Hexagonal Architecture
//!
//! Handlers use dependency injection to receive infrastructure adapters
//! via the [`HandlerDependencies`] struct. This allows the business logic
//! to be tested independently of infrastructure concerns.

use anyhow::Result;
use chrono::Utc;
use minibox_lib::domain::{
    ContainerSpawnConfig, DomainError, DynContainerRuntime, DynFilesystemProvider,
    DynImageRegistry, DynResourceLimiter, ResourceConfig,
};
use minibox_lib::protocol::{ContainerInfo, DaemonResponse};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

use crate::state::{ContainerRecord, DaemonState};

// ---------------------------------------------------------------------------
// Handler Dependencies (Dependency Injection)
// ---------------------------------------------------------------------------

/// Dependencies injected into request handlers.
///
/// This struct bundles all infrastructure adapters (trait implementations)
/// that handlers need to perform their operations. Following hexagonal
/// architecture principles, handlers depend on trait abstractions rather
/// than concrete implementations.
///
/// # Usage
///
/// Created once in the composition root (main.rs) and passed to all handlers:
///
/// ```rust,ignore
/// use minibox_lib::adapters::{DockerHubRegistry, OverlayFilesystem, CgroupV2Limiter, LinuxNamespaceRuntime};
///
/// let deps = Arc::new(HandlerDependencies {
///     registry: Arc::new(DockerHubRegistry::new(store)?),
///     filesystem: Arc::new(OverlayFilesystem),
///     resource_limiter: Arc::new(CgroupV2Limiter),
///     runtime: Arc::new(LinuxNamespaceRuntime),
///     containers_base: PathBuf::from("/var/lib/minibox/containers"),
///     run_containers_base: PathBuf::from("/run/minibox/containers"),
/// });
/// ```
#[derive(Clone)]
pub struct HandlerDependencies {
    /// Image registry for pulling container images.
    pub registry: DynImageRegistry,
    /// Filesystem provider for setting up container rootfs.
    pub filesystem: DynFilesystemProvider,
    /// Resource limiter for enforcing cgroup limits.
    pub resource_limiter: DynResourceLimiter,
    /// Container runtime for spawning isolated processes.
    pub runtime: DynContainerRuntime,
    /// Base directory for persistent container data (overlay dirs).
    pub containers_base: PathBuf,
    /// Base directory for runtime container state (PID files).
    pub run_containers_base: PathBuf,
}

// ─── Run ────────────────────────────────────────────────────────────────────

/// Create and start a new container from `image:tag`, executing `command`.
///
/// Responses are sent via `tx`.  Non-ephemeral runs send exactly one message.
/// Ephemeral runs (Linux-only) send zero or more `ContainerOutput` messages
/// followed by one terminal `ContainerStopped` message.
#[allow(clippy::too_many_arguments)]
pub async fn handle_run(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    #[allow(unused_variables)] ephemeral: bool,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    #[cfg(target_os = "linux")]
    if ephemeral {
        handle_run_streaming(
            image,
            tag,
            command,
            memory_limit_bytes,
            cpu_weight,
            state,
            deps,
            tx,
        )
        .await;
        return;
    }

    // Non-ephemeral (or non-Linux): single response.
    let response = match run_inner(
        image,
        tag,
        command,
        memory_limit_bytes,
        cpu_weight,
        state,
        deps,
    )
    .await
    {
        Ok(id) => DaemonResponse::ContainerCreated { id },
        Err(e) => {
            error!("handle_run error: {e:#}");
            DaemonResponse::Error {
                message: format!("{e:#}"),
            }
        }
    };
    let _ = tx.send(response).await;
}

/// Streaming ephemeral run: sends `ContainerOutput` chunks then `ContainerStopped`.
///
/// The container stdout+stderr are forwarded via the channel until EOF, then
/// the exit code is reported.
#[cfg(target_os = "linux")]
async fn handle_run_streaming(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    use minibox_lib::protocol::OutputStreamKind;
    use std::os::fd::IntoRawFd;

    // Build the container ID and rootfs via the shared inner setup, but we need
    // capture_output=true. We inline a variant of run_inner here.
    let result = run_inner_capture(
        image,
        tag,
        command,
        memory_limit_bytes,
        cpu_weight,
        Arc::clone(&state),
        Arc::clone(&deps),
    )
    .await;

    let (container_id, pid, output_reader) = match result {
        Ok(triple) => triple,
        Err(e) => {
            error!("handle_run_streaming setup error: {e:#}");
            let _ = tx
                .send(DaemonResponse::Error {
                    message: format!("{e:#}"),
                })
                .await;
            return;
        }
    };

    // Spawn blocking task to drain the pipe and forward chunks.
    let tx_clone = tx.clone();
    // SAFETY: OwnedFd is not Send on all platforms, so we transfer ownership via raw fd.
    // The OwnedFd is consumed by into_raw_fd() (no drop), and from_raw_fd() inside the
    // closure takes sole ownership. No other code touches reader_raw after this point.
    let reader_raw = output_reader.into_raw_fd();
    let drain_handle = tokio::task::spawn_blocking(move || {
        use std::io::Read;

        // SAFETY: we own this fd from the pipe created in spawn_container_process.
        let mut file = unsafe { std::fs::File::from_raw_fd(reader_raw) };
        let mut buf = [0u8; 4096];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break, // EOF — child exited and closed its write end.
                Ok(n) => {
                    use base64::Engine;
                    let encoded = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                    let _ = tx_clone.blocking_send(DaemonResponse::ContainerOutput {
                        stream: OutputStreamKind::Stdout,
                        data: encoded,
                    });
                }
                Err(e) => {
                    warn!(pid = pid, error = %e, "pipe drain: read error");
                    break;
                }
            }
        }
    });

    // Wait for the child process to exit.
    use minibox_lib::container::process::wait_for_exit;
    let exit_code = tokio::task::spawn_blocking(move || wait_for_exit(pid))
        .await
        .unwrap_or(Ok(-1))
        .unwrap_or(-1);

    // Wait for drain to finish before sending ContainerStopped
    // so all output is flushed before the terminal message.
    if let Err(e) = drain_handle.await {
        warn!(pid = pid, "pipe drain task panicked: {:?}", e);
    }

    // Auto-remove ephemeral container state.
    state.remove_container(&container_id).await;

    let _ = tx
        .send(DaemonResponse::ContainerStopped { exit_code })
        .await;
}

/// Like `run_inner` but returns `(container_id, pid, output_reader)` with
/// `capture_output = true` in the spawn config.
///
/// Only compiled on Linux because the output pipe requires Linux primitives.
#[cfg(target_os = "linux")]
async fn run_inner_capture(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> Result<(String, u32, std::os::fd::OwnedFd)> {
    let tag = tag.unwrap_or_else(|| "latest".to_string());

    let full_image = if image.contains('/') {
        image.clone()
    } else {
        format!("library/{image}")
    };

    if !deps.registry.has_image(&full_image, &tag).await {
        info!("image {full_image}:{tag} not cached, pulling…");
        deps.registry
            .pull_image(&full_image, &tag)
            .await
            .map_err(|e| DomainError::ImagePullFailed {
                image: full_image.clone(),
                tag: tag.clone(),
                source: e,
            })?;
    }

    let layer_dirs = deps.registry.get_image_layers(&full_image, &tag)?;
    if layer_dirs.is_empty() {
        return Err(DomainError::EmptyImage {
            name: full_image.clone(),
            tag: tag.clone(),
        }
        .into());
    }

    let id = Uuid::new_v4()
        .to_string()
        .replace('-', "")
        .chars()
        .take(16)
        .collect::<String>();

    if state.get_container(&id).await.is_some() {
        return Err(DomainError::InvalidConfig(format!(
            "container ID collision (extremely rare): {id}"
        ))
        .into());
    }

    let container_dir = deps.containers_base.join(&id);
    let run_dir = deps.run_containers_base.join(&id);

    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        builder.recursive(true);
        builder.create(&container_dir)?;
        builder.create(&run_dir)?;
    }

    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(&container_dir)?;
        std::fs::create_dir_all(&run_dir)?;
    }

    let merged_dir = deps.filesystem.setup_rootfs(&layer_dirs, &container_dir)?;

    let resource_config = ResourceConfig {
        memory_limit_bytes,
        cpu_weight,
        pids_max: Some(1024),
        io_max_bytes_per_sec: None,
    };
    let cgroup_dir_str = deps.resource_limiter.create(&id, &resource_config)?;
    let cgroup_dir = PathBuf::from(cgroup_dir_str);

    let image_label = format!("{image}:{tag}");
    let command_str = command.join(" ");
    let record = ContainerRecord {
        info: ContainerInfo {
            id: id.clone(),
            image: image_label,
            command: command_str,
            state: "Created".to_string(),
            created_at: Utc::now().to_rfc3339(),
            pid: None,
        },
        pid: None,
        rootfs_path: merged_dir.clone(),
        cgroup_path: cgroup_dir.clone(),
    };
    state.add_container(record).await;

    let spawn_command = command
        .first()
        .cloned()
        .unwrap_or_else(|| "/bin/sh".to_string());
    let spawn_args = command.iter().skip(1).cloned().collect();
    let spawn_config = ContainerSpawnConfig {
        rootfs: merged_dir.clone(),
        command: spawn_command,
        args: spawn_args,
        env: vec![
            "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
            "TERM=xterm".to_string(),
        ],
        cgroup_path: cgroup_dir.clone(),
        hostname: format!("minibox-{}", &id[..8]),
        capture_output: true,
    };

    let _spawn_permit = state
        .spawn_semaphore
        .acquire()
        .await
        .expect("semaphore closed");

    let spawn_result = deps.runtime.spawn_process(&spawn_config).await?;

    let pid = spawn_result.pid;
    let output_reader = spawn_result.output_reader.ok_or_else(|| {
        anyhow::anyhow!("capture_output=true but runtime returned no output_reader")
    })?;

    // Write PID file and update state.
    let pid_file = deps.run_containers_base.join(&id).join("pid");
    let _ = std::fs::write(&pid_file, pid.to_string());
    state.set_container_pid(&id, pid).await;

    Ok((id, pid, output_reader))
}

async fn run_inner(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> Result<String> {
    let tag = tag.unwrap_or_else(|| "latest".to_string());

    // Normalise image name: bare "alpine" → "library/alpine"
    let full_image = if image.contains('/') {
        image.clone()
    } else {
        format!("library/{image}")
    };

    // Pull image if not cached (using injected registry trait).
    if !deps.registry.has_image(&full_image, &tag).await {
        info!("image {full_image}:{tag} not cached, pulling…");
        deps.registry
            .pull_image(&full_image, &tag)
            .await
            .map_err(|e| DomainError::ImagePullFailed {
                image: full_image.clone(),
                tag: tag.clone(),
                source: e,
            })?;
    }

    let layer_dirs = deps.registry.get_image_layers(&full_image, &tag)?;
    if layer_dirs.is_empty() {
        return Err(DomainError::EmptyImage {
            name: full_image.clone(),
            tag: tag.clone(),
        }
        .into());
    }

    // SECURITY: Generate a 16-char container ID from UUID to prevent collisions.
    // 16 hex chars = 64 bits, birthday paradox collision after ~4 billion containers.
    // We also check for collisions below.
    let id = Uuid::new_v4()
        .to_string()
        .replace('-', "")
        .chars()
        .take(16)
        .collect::<String>();

    // SECURITY: Verify no collision with existing containers
    if state.get_container(&id).await.is_some() {
        return Err(DomainError::InvalidConfig(format!(
            "container ID collision (extremely rare): {id}"
        ))
        .into());
    }

    let container_dir = deps.containers_base.join(&id);
    let run_dir = deps.run_containers_base.join(&id);

    // SECURITY: Create container directories with restricted permissions (0700)
    // to prevent unauthorized access to container data
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;

        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700); // Owner (root) only
        builder.recursive(true);
        builder.create(&container_dir)?;
        builder.create(&run_dir)?;
    }

    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(&container_dir)?;
        std::fs::create_dir_all(&run_dir)?;
    }

    // Setup overlayfs (using injected filesystem trait).
    let merged_dir_from_overlay = deps.filesystem.setup_rootfs(&layer_dirs, &container_dir)?;

    // Setup cgroup (using injected resource limiter trait).
    let resource_config = ResourceConfig {
        memory_limit_bytes,
        cpu_weight,
        pids_max: Some(1024), // Default PID limit for security
        io_max_bytes_per_sec: None,
    };
    let cgroup_dir_str = deps.resource_limiter.create(&id, &resource_config)?;
    let cgroup_dir = PathBuf::from(cgroup_dir_str);

    // Build ContainerRecord in Created state; updated to Running once the
    // child PID is known.
    let image_label = format!("{image}:{tag}");
    let command_str = command.join(" ");
    let record = ContainerRecord {
        info: ContainerInfo {
            id: id.clone(),
            image: image_label.clone(),
            command: command_str,
            state: "Created".to_string(),
            created_at: Utc::now().to_rfc3339(),
            pid: None,
        },
        pid: None,
        rootfs_path: merged_dir_from_overlay.clone(),
        cgroup_path: cgroup_dir.clone(),
    };
    state.add_container(record).await;

    // Build the ContainerSpawnConfig for the runtime.
    let spawn_command = command
        .first()
        .cloned()
        .unwrap_or_else(|| "/bin/sh".to_string());
    let spawn_args = command.iter().skip(1).cloned().collect();
    let spawn_config = ContainerSpawnConfig {
        rootfs: merged_dir_from_overlay.clone(),
        command: spawn_command,
        args: spawn_args,
        env: vec![
            "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
            "TERM=xterm".to_string(),
        ],
        cgroup_path: cgroup_dir.clone(),
        hostname: format!("minibox-{}", &id[..8]),
        capture_output: false,
    };

    // SECURITY: Acquire semaphore permit to limit concurrent spawns
    // This prevents fork bomb attacks from overwhelming the system
    let _spawn_permit = state
        .spawn_semaphore
        .acquire()
        .await
        .expect("semaphore closed");

    // Spawn the container process (using injected runtime trait).
    let id_clone = id.clone();
    let state_clone = Arc::clone(&state);
    let runtime_clone = Arc::clone(&deps.runtime);
    let run_containers_base_clone = deps.run_containers_base.clone();

    tokio::task::spawn(async move {
        // Permit is held until this task completes (via _spawn_permit drop)
        match runtime_clone.spawn_process(&spawn_config).await {
            Ok(spawn_result) => {
                let pid = spawn_result.pid;
                info!("container {id_clone} started with PID {pid}");

                // Write PID file.
                let pid_file = run_containers_base_clone.join(&id_clone).join("pid");
                let _ = std::fs::write(&pid_file, pid.to_string());

                state_clone.set_container_pid(&id_clone, pid).await;

                // Wait for the process to exit in a background task.
                let state_wait = Arc::clone(&state_clone);
                let id_wait = id_clone.clone();
                tokio::task::spawn_blocking(move || {
                    daemon_wait_for_exit(pid, &id_wait, state_wait);
                });
            }
            Err(e) => {
                error!("failed to spawn container {id_clone}: {e:#}");
                state_clone
                    .update_container_state(&id_clone, "Failed")
                    .await;
            }
        }
    });

    Ok(id)
}

/// Block until the container PID exits, then update its state in the daemon.
#[cfg(unix)]
fn daemon_wait_for_exit(pid: u32, id: &str, state: Arc<DaemonState>) {
    use nix::sys::wait::{WaitStatus, waitpid};
    use nix::unistd::Pid;
    let nix_pid = Pid::from_raw(pid as i32);
    match waitpid(nix_pid, None) {
        Ok(WaitStatus::Exited(_, code)) => {
            info!("container {id} exited with code {code}");
        }
        Ok(WaitStatus::Signaled(_, sig, _)) => {
            info!("container {id} killed by signal {sig}");
        }
        Ok(other) => {
            info!("container {id} wait status: {other:?}");
        }
        Err(e) => {
            warn!("waitpid for container {id} error: {e}");
        }
    }

    // Mark stopped; bridge async state update from sync context.
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.block_on(state.update_container_state(id, "Stopped"));
        }
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("one-shot runtime");
            rt.block_on(state.update_container_state(id, "Stopped"));
        }
    }
}

#[cfg(windows)]
fn daemon_wait_for_exit(_pid: u32, _id: &str, _state: Arc<DaemonState>) {
    // No-op on Windows. Container stays "Running" until explicit stop/remove.
}

#[cfg(not(any(unix, windows)))]
fn daemon_wait_for_exit(_pid: u32, _id: &str, _state: Arc<DaemonState>) {
    // No-op on this platform.
}

// ─── Stop ───────────────────────────────────────────────────────────────────

/// Send SIGTERM to a container, then SIGKILL after 10 seconds if needed.
pub async fn handle_stop(id: String, state: Arc<DaemonState>) -> DaemonResponse {
    match stop_inner(&id, &state).await {
        Ok(()) => DaemonResponse::Success {
            message: format!("container {id} stopped"),
        },
        Err(e) => {
            error!("handle_stop error: {e:#}");
            DaemonResponse::Error {
                message: format!("{e:#}"),
            }
        }
    }
}

#[cfg(unix)]
async fn stop_inner(id: &str, state: &Arc<DaemonState>) -> Result<()> {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    let record = state
        .get_container(id)
        .await
        .ok_or_else(|| DomainError::ContainerNotFound { id: id.to_string() })?;

    let pid = record
        .pid
        .ok_or_else(|| anyhow::anyhow!("container {id} has no PID (not running?)"))?;

    let nix_pid = Pid::from_raw(pid as i32);

    info!("sending SIGTERM to container {id} (PID {pid})");
    kill(nix_pid, Signal::SIGTERM).ok();

    // Wait up to 10 s for the process to exit.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if kill(nix_pid, None).is_err() {
            // ESRCH — process is gone.
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            warn!("container {id} did not exit in 10 s, sending SIGKILL");
            kill(nix_pid, Signal::SIGKILL).ok();
            break;
        }
    }

    state.update_container_state(id, "Stopped").await;
    Ok(())
}

#[cfg(windows)]
async fn stop_inner(id: &str, _state: &Arc<DaemonState>) -> Result<()> {
    anyhow::bail!(
        "handle_stop not yet implemented on Windows for container {id} \
         — use the HCS/WSL2 adapter stop path"
    )
}

#[cfg(not(any(unix, windows)))]
async fn stop_inner(id: &str, _state: &Arc<DaemonState>) -> Result<()> {
    anyhow::bail!("handle_stop not supported on this platform for container {id}")
}

// ─── Remove ─────────────────────────────────────────────────────────────────

/// Clean up a stopped container: unmount overlay, delete dirs, remove cgroup.
pub async fn handle_remove(
    id: String,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    match remove_inner(&id, &state, &deps).await {
        Ok(()) => DaemonResponse::Success {
            message: format!("container {id} removed"),
        },
        Err(e) => {
            error!("handle_remove error: {e:#}");
            DaemonResponse::Error {
                message: format!("{e:#}"),
            }
        }
    }
}

async fn remove_inner(
    id: &str,
    state: &Arc<DaemonState>,
    deps: &Arc<HandlerDependencies>,
) -> Result<()> {
    let record = state
        .get_container(id)
        .await
        .ok_or_else(|| DomainError::ContainerNotFound { id: id.to_string() })?;

    if record.info.state == "Running" {
        return Err(DomainError::AlreadyRunning { id: id.to_string() }.into());
    }

    // Unmount overlay (using injected filesystem trait).
    let container_dir = deps.containers_base.join(id);
    if container_dir.exists()
        && let Err(e) = deps.filesystem.cleanup(&container_dir)
    {
        warn!("cleanup_mounts for {id}: {e}");
    }

    // Remove runtime state directory.
    let run_dir = deps.run_containers_base.join(id);
    if run_dir.exists() {
        std::fs::remove_dir_all(&run_dir).ok();
    }

    // Cleanup cgroup (using injected resource limiter trait).
    if let Err(e) = deps.resource_limiter.cleanup(id) {
        warn!("cleanup cgroup for {id}: {e}");
    }

    state.remove_container(id).await;
    Ok(())
}

// ─── List ───────────────────────────────────────────────────────────────────

/// Return all known containers.
pub async fn handle_list(state: Arc<DaemonState>) -> DaemonResponse {
    let containers = state.list_containers().await;
    DaemonResponse::ContainerList { containers }
}

// ─── Pull ───────────────────────────────────────────────────────────────────

/// Pull an image from Docker Hub and cache it locally.
#[instrument(skip(_state, deps), fields(image = %image, tag = ?tag))]
pub async fn handle_pull(
    image: String,
    tag: Option<String>,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    let tag = tag.unwrap_or_else(|| "latest".to_string());
    let full_image = if image.contains('/') {
        image.clone()
    } else {
        format!("library/{image}")
    };

    // Pull image (using injected registry trait).
    match deps.registry.pull_image(&full_image, &tag).await {
        Ok(_metadata) => DaemonResponse::Success {
            message: format!("pulled {image}:{tag}"),
        },
        Err(e) => {
            error!("handle_pull error: {e:#}");
            DaemonResponse::Error {
                message: format!("{e:#}"),
            }
        }
    }
}
