//! Request handlers for each daemon operation.
//!
//! Each public function corresponds to one `DaemonRequest` variant and
//! returns a `DaemonResponse`.  Errors are caught and returned as
//! `DaemonResponse::Error` so the daemon never panics on bad input.

use anyhow::Result;
use chrono::Utc;
use minibox_lib::container::cgroups::{CgroupConfig, CgroupManager};
use minibox_lib::container::filesystem;
use minibox_lib::container::namespace::NamespaceConfig;
use minibox_lib::container::process::{spawn_container_process, ContainerConfig};
use minibox_lib::image::registry::RegistryClient;
use minibox_lib::protocol::{ContainerInfo, DaemonResponse};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::state::{ContainerRecord, DaemonState};

/// Directories used by the daemon.
const CONTAINERS_BASE: &str = "/var/lib/minibox/containers";
const RUN_CONTAINERS_BASE: &str = "/run/minibox/containers";

// ─── Run ────────────────────────────────────────────────────────────────────

/// Create and start a new container from `image:tag`, executing `command`.
pub async fn handle_run(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    state: Arc<DaemonState>,
) -> DaemonResponse {
    match run_inner(
        image,
        tag,
        command,
        memory_limit_bytes,
        cpu_weight,
        state,
    )
    .await
    {
        Ok(id) => DaemonResponse::ContainerCreated { id },
        Err(e) => {
            error!("handle_run error: {:#}", e);
            DaemonResponse::Error {
                message: format!("{:#}", e),
            }
        }
    }
}

async fn run_inner(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    state: Arc<DaemonState>,
) -> Result<String> {
    let tag = tag.unwrap_or_else(|| "latest".to_string());

    // Normalise image name: bare "alpine" → "library/alpine"
    let full_image = if image.contains('/') {
        image.clone()
    } else {
        format!("library/{}", image)
    };

    // Pull image if not cached.
    if !state.image_store.has_image(&full_image, &tag) {
        info!("image {}:{} not cached, pulling…", full_image, tag);
        let client = RegistryClient::new()?;
        client
            .pull_image(&full_image, &tag, &state.image_store)
            .await?;
    }

    let layer_dirs = state.image_store.get_image_layers(&full_image, &tag)?;
    if layer_dirs.is_empty() {
        anyhow::bail!("image {}:{} has no layers", full_image, tag);
    }

    // Generate a short (12-char) container ID from a UUID.
    let id = Uuid::new_v4()
        .to_string()
        .replace('-', "")
        .chars()
        .take(12)
        .collect::<String>();

    // Create container directory layout.
    let container_dir = PathBuf::from(CONTAINERS_BASE).join(&id);
    std::fs::create_dir_all(&container_dir)?;

    // Runtime state directory.
    let run_dir = PathBuf::from(RUN_CONTAINERS_BASE).join(&id);
    std::fs::create_dir_all(&run_dir)?;

    // Setup overlayfs: creates upper/, work/, merged/ under container_dir.
    let merged_dir_from_overlay =
        filesystem::setup_overlay(&layer_dirs, &container_dir)?;

    // Setup cgroup using the CgroupManager API.
    let cgroup_config = CgroupConfig {
        memory_limit_bytes,
        cpu_weight,
    };
    let cgroup_manager = CgroupManager::new(&id, cgroup_config);
    cgroup_manager.create()?;
    let cgroup_dir = cgroup_manager.cgroup_path.clone();

    // Build ContainerRecord in Created state; updated to Running once the
    // child PID is known.
    let image_label = format!("{}:{}", image, tag);
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

    // Build the ContainerConfig for spawn_container_process.
    let spawn_command = command.first().cloned().unwrap_or_else(|| "/bin/sh".to_string());
    let spawn_args = command.iter().skip(1).cloned().collect();
    let container_config = ContainerConfig {
        rootfs: merged_dir_from_overlay.clone(),
        command: spawn_command,
        args: spawn_args,
        env: vec![
            "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
            "TERM=xterm".to_string(),
        ],
        namespace_config: NamespaceConfig::all(),
        cgroup_path: cgroup_dir.clone(),
        hostname: format!("minibox-{}", &id[..8]),
    };

    // Spawn the container process in a blocking thread (clone/fork is sync).
    let id_clone = id.clone();
    let state_clone = Arc::clone(&state);

    tokio::task::spawn(async move {
        match tokio::task::spawn_blocking(move || {
            spawn_container_process(container_config)
        })
        .await
        {
            Ok(Ok(pid)) => {
                info!("container {} started with PID {}", id_clone, pid);

                // Write PID file.
                let pid_file =
                    PathBuf::from(RUN_CONTAINERS_BASE).join(&id_clone).join("pid");
                let _ = std::fs::write(&pid_file, pid.to_string());

                state_clone.set_container_pid(&id_clone, pid).await;

                // Wait for the process to exit in a background task.
                let state_wait = Arc::clone(&state_clone);
                let id_wait = id_clone.clone();
                tokio::task::spawn_blocking(move || {
                    daemon_wait_for_exit(pid, &id_wait, state_wait);
                });
            }
            Ok(Err(e)) => {
                error!("failed to spawn container {}: {:#}", id_clone, e);
                state_clone
                    .update_container_state(&id_clone, "Failed")
                    .await;
            }
            Err(e) => {
                error!("spawn_blocking join error for {}: {}", id_clone, e);
                state_clone
                    .update_container_state(&id_clone, "Failed")
                    .await;
            }
        }
    });

    Ok(id)
}

/// Block until the container PID exits, then update its state in the daemon.
fn daemon_wait_for_exit(pid: u32, id: &str, state: Arc<DaemonState>) {
    use nix::sys::wait::{waitpid, WaitStatus};
    let nix_pid = Pid::from_raw(pid as i32);
    match waitpid(nix_pid, None) {
        Ok(WaitStatus::Exited(_, code)) => {
            info!("container {} exited with code {}", id, code);
        }
        Ok(WaitStatus::Signaled(_, sig, _)) => {
            info!("container {} killed by signal {}", id, sig);
        }
        Ok(other) => {
            info!("container {} wait status: {:?}", id, other);
        }
        Err(e) => {
            warn!("waitpid for container {} error: {}", id, e);
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

// ─── Stop ───────────────────────────────────────────────────────────────────

/// Send SIGTERM to a container, then SIGKILL after 10 seconds if needed.
pub async fn handle_stop(id: String, state: Arc<DaemonState>) -> DaemonResponse {
    match stop_inner(&id, &state).await {
        Ok(()) => DaemonResponse::Success {
            message: format!("container {} stopped", id),
        },
        Err(e) => {
            error!("handle_stop error: {:#}", e);
            DaemonResponse::Error {
                message: format!("{:#}", e),
            }
        }
    }
}

async fn stop_inner(id: &str, state: &Arc<DaemonState>) -> Result<()> {
    let record = state
        .get_container(id)
        .await
        .ok_or_else(|| anyhow::anyhow!("container {} not found", id))?;

    let pid = record
        .pid
        .ok_or_else(|| anyhow::anyhow!("container {} has no PID (not running?)", id))?;

    let nix_pid = Pid::from_raw(pid as i32);

    info!("sending SIGTERM to container {} (PID {})", id, pid);
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
            warn!("container {} did not exit in 10 s, sending SIGKILL", id);
            kill(nix_pid, Signal::SIGKILL).ok();
            break;
        }
    }

    state.update_container_state(id, "Stopped").await;
    Ok(())
}

// ─── Remove ─────────────────────────────────────────────────────────────────

/// Clean up a stopped container: unmount overlay, delete dirs, remove cgroup.
pub async fn handle_remove(id: String, state: Arc<DaemonState>) -> DaemonResponse {
    match remove_inner(&id, &state).await {
        Ok(()) => DaemonResponse::Success {
            message: format!("container {} removed", id),
        },
        Err(e) => {
            error!("handle_remove error: {:#}", e);
            DaemonResponse::Error {
                message: format!("{:#}", e),
            }
        }
    }
}

async fn remove_inner(id: &str, state: &Arc<DaemonState>) -> Result<()> {
    let record = state
        .get_container(id)
        .await
        .ok_or_else(|| anyhow::anyhow!("container {} not found", id))?;

    if record.info.state == "Running" {
        anyhow::bail!("container {} is still running; stop it first", id);
    }

    // Unmount overlay.
    let container_dir = PathBuf::from(CONTAINERS_BASE).join(id);
    if container_dir.exists() {
        if let Err(e) = filesystem::cleanup_mounts(&container_dir) {
            warn!("cleanup_mounts for {}: {}", id, e);
        }
    }

    // Remove runtime state directory.
    let run_dir = PathBuf::from(RUN_CONTAINERS_BASE).join(id);
    if run_dir.exists() {
        std::fs::remove_dir_all(&run_dir).ok();
    }

    // Cleanup cgroup.
    let cgroup_manager = CgroupManager::new(id, CgroupConfig::default());
    if let Err(e) = cgroup_manager.cleanup() {
        warn!("cleanup cgroup for {}: {}", id, e);
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
pub async fn handle_pull(
    image: String,
    tag: Option<String>,
    state: Arc<DaemonState>,
) -> DaemonResponse {
    let tag = tag.unwrap_or_else(|| "latest".to_string());
    let full_image = if image.contains('/') {
        image.clone()
    } else {
        format!("library/{}", image)
    };

    let client = match RegistryClient::new() {
        Ok(c) => c,
        Err(e) => {
            return DaemonResponse::Error {
                message: format!("failed to create registry client: {}", e),
            };
        }
    };

    match client.pull_image(&full_image, &tag, &state.image_store).await {
        Ok(()) => DaemonResponse::Success {
            message: format!("pulled {}:{}", image, tag),
        },
        Err(e) => {
            error!("handle_pull error: {:#}", e);
            DaemonResponse::Error {
                message: format!("{:#}", e),
            }
        }
    }
}
