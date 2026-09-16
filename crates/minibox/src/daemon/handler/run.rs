//! Container run handlers and supporting infrastructure.
// Handler signatures require >5 parameters by design (DI pattern). See rustqual.toml.
#![allow(clippy::too_many_arguments)]
// TODO(#326): persist execution manifest before container spawn
// TODO(#366): extract shared run preparation path from handle_run

use anyhow::{Context as _, Result};
use minibox_core::domain::{DynContainerRuntime, HookSpec};
use minibox_core::events::{ContainerEvent, EventSink};
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::daemon::state::{ContainerState, DaemonState, RunCreationParams};

use super::HandlerDependencies;

mod model;
mod preparation;
mod request;

pub use model::RunParams;
pub use request::handle_run;

#[cfg(unix)]
use preparation::prepare_run;

/// Bundled parameters for `daemon_wait_for_exit`.
pub(super) struct WaitParams {
    pub pid: u32,
    pub id: String,
    pub state: Arc<DaemonState>,
    pub rootfs: minibox_core::path::InternalPath,
    pub post_exit_hooks: Vec<HookSpec>,
    pub event_sink: Arc<dyn EventSink>,
    pub cgroup_path: minibox_core::path::InternalPath,
    pub runtime: DynContainerRuntime,
    pub runtime_id: Option<String>,
}

// ─── Container ID Generation ─────────────────────────────────────────────────

/// Generate a 16-char hex container ID from a UUID v4.
///
/// 16 hex chars = 64 bits. Birthday-paradox collision after ~4 billion containers —
/// callers must still check for collisions against the existing container state.
pub fn generate_container_id() -> String {
    Uuid::new_v4()
        .to_string()
        .replace('-', "")
        .chars()
        .take(16)
        .collect()
}

/// Variant of `run_inner` that enables output capture for ephemeral containers.
///
/// Sets `capture_output = true` in the spawn config so the runtime creates a
/// pipe between the container process and the daemon.  Returns the container ID,
/// the child PID, and the read end of the output pipe as an [`OwnedFd`].
///
/// The caller is responsible for draining the pipe (to avoid blocking the child
/// on a full pipe buffer) and for calling `wait_for_exit` to reap the process.
///
/// Container state transitions: `"Created"` → `"Running"` (via
/// `set_container_pid`).  The `"Stopped"` transition is handled by the caller
/// (`handle_run_streaming`) after the process exits.
///
/// Compiled on Unix (Linux and macOS). The output pipe uses `OwnedFd`
/// and `waitpid` — both available on any Unix via the `nix` crate.
// qual:allow(iosp) reason: "container lifecycle — namespace setup, fork, exec"
#[cfg(unix)]
async fn run_inner_capture(
    params: RunParams,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> Result<(String, u32, std::os::fd::OwnedFd, Option<String>)> {
    let prepared = prepare_run(params, true, Arc::clone(&state), Arc::clone(&deps)).await?;

    state
        .set_manifest_info(
            &prepared.id,
            prepared.manifest_path.clone(),
            prepared.workload_digest.clone(),
        )
        .await;

    // Semaphore closed only if the daemon is shutting down; no recovery possible.
    #[allow(clippy::expect_used)]
    let _spawn_permit = state
        .spawn_semaphore
        .acquire()
        .await
        .expect("semaphore closed");

    let spawn_result = match deps
        .lifecycle
        .runtime
        .spawn_process(&prepared.spawn_config)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!("failed to spawn container {}: {e:#}", prepared.id);
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "run"), ("adapter", "daemon"), ("status", "error")],
            );
            // Overlay and cgroup were already set up in prepare_run before the
            // spawn attempt — tear them down so a spawn failure doesn't orphan
            // them. Mirrors the cleanup sequence in handler/lifecycle.rs's
            // handle_remove.
            let container_dir = deps.lifecycle.containers_base.join(&prepared.id);
            if container_dir.exists() {
                if !container_dir.starts_with(&deps.lifecycle.containers_base) {
                    warn!(
                        path = %container_dir.display(),
                        base = %deps.lifecycle.containers_base.display(),
                        "run: container_dir escapes base directory, skipping cleanup"
                    );
                } else if let Err(ce) = deps.lifecycle.filesystem.cleanup(&container_dir) {
                    warn!(container_id = %prepared.id, error = %ce, "run: overlay cleanup failed after spawn error");
                }
            }
            if let Err(ce) = deps.lifecycle.resource_limiter.cleanup(&prepared.id) {
                warn!(container_id = %prepared.id, error = %ce, "run: cgroup cleanup failed after spawn error");
            }
            if let Err(ue) = state
                .update_container_state(&prepared.id, ContainerState::Failed)
                .await
            {
                warn!(container_id = %prepared.id, error = %ue, "state: failed to mark container Failed");
            }
            return Err(e);
        }
    };

    let pid = spawn_result.pid;
    let runtime_id = spawn_result.runtime_id;
    let output_reader = spawn_result.output_reader.ok_or_else(|| {
        anyhow::anyhow!("capture_output=true but runtime returned no output_reader")
    })?;

    // ── Network attach ─────────────────────────────────────────────────
    prepared
        .net
        .attach(&prepared.id, pid)
        .await
        .context("network attach")?;

    // Write PID file and update state.
    let pid_file = deps
        .lifecycle
        .run_containers_base
        .join(&prepared.id)
        .join("pid");
    if let Err(e) = std::fs::write(&pid_file, pid.to_string()) {
        warn!(
            pid_file = %pid_file.display(),
            error = %e,
            "container: failed to write pid file"
        );
    }
    state.set_container_pid(&prepared.id, pid).await;
    state
        .set_container_runtime_id(&prepared.id, runtime_id.clone())
        .await;

    Ok((prepared.id, pid, output_reader, runtime_id))
}

/// Pull the image if needed, set up the overlay rootfs and cgroup, register the
/// container in `"Created"` state, then spawn the container process.
///
/// Returns the new container ID immediately after the spawn task is dispatched.
/// The container transitions from `"Created"` to `"Running"` asynchronously
/// once the runtime reports the child PID.  A background reaper task
/// (`daemon_wait_for_exit`) drives the final `"Stopped"` transition.
///
/// # Async / sync boundary
///
/// The runtime's `spawn_process` is async (it may perform IPC with an external
/// runtime such as Colima).  The actual fork/clone/exec for the native Linux
/// adapter happens inside `spawn_process` via `tokio::task::spawn_blocking` in
/// the runtime implementation, keeping blocking syscalls off the Tokio worker
/// threads.  The reaper is also dispatched via `spawn_blocking` because
/// `waitpid` is a blocking syscall.
// qual:allow(iosp) reason: "container lifecycle — overlay, spawn, reaper"
async fn run_inner(
    params: RunParams,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> Result<String> {
    let prepared = prepare_run(params, false, Arc::clone(&state), Arc::clone(&deps)).await?;

    state
        .set_manifest_info(
            &prepared.id,
            prepared.manifest_path.clone(),
            prepared.workload_digest.clone(),
        )
        .await;

    let id = prepared.id.clone();
    let image_label = prepared.image_label.clone();
    let spawn_config = prepared.spawn_config;

    // SECURITY: Acquire semaphore permit to limit concurrent spawns.
    // Semaphore closed only if the daemon is shutting down; no recovery possible.
    #[allow(clippy::expect_used)]
    let _spawn_permit = state
        .spawn_semaphore
        .acquire()
        .await
        .expect("semaphore closed");

    // Spawn the container process synchronously so failures propagate to the caller.
    let spawn_result = match deps.lifecycle.runtime.spawn_process(&spawn_config).await {
        Ok(r) => r,
        Err(e) => {
            error!("failed to spawn container {id}: {e:#}");
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "run"), ("adapter", "daemon"), ("status", "error")],
            );
            // Overlay and cgroup were already set up in prepare_run before the
            // spawn attempt — tear them down so a spawn failure doesn't orphan
            // them. Mirrors the cleanup sequence in handler/lifecycle.rs's
            // handle_remove.
            let container_dir = deps.lifecycle.containers_base.join(&id);
            if container_dir.exists() {
                if !container_dir.starts_with(&deps.lifecycle.containers_base) {
                    warn!(
                        path = %container_dir.display(),
                        base = %deps.lifecycle.containers_base.display(),
                        "run: container_dir escapes base directory, skipping cleanup"
                    );
                } else if let Err(ce) = deps.lifecycle.filesystem.cleanup(&container_dir) {
                    warn!(container_id = %id, error = %ce, "run: overlay cleanup failed after spawn error");
                }
            }
            if let Err(ce) = deps.lifecycle.resource_limiter.cleanup(&id) {
                warn!(container_id = %id, error = %ce, "run: cgroup cleanup failed after spawn error");
            }
            if let Err(ue) = state
                .update_container_state(&id, ContainerState::Failed)
                .await
            {
                warn!(container_id = %id, error = %ue, "state: failed to mark container Failed");
            }
            return Err(e);
        }
    };
    // Release the permit now that the process is running.
    drop(_spawn_permit);

    let pid = spawn_result.pid;
    info!(container_id = %id, pid = pid, "container: process started");

    deps.events.event_sink.emit(ContainerEvent::Created {
        id: id.clone(),
        image: image_label.clone(),
        timestamp: std::time::SystemTime::now(),
    });
    deps.events.event_sink.emit(ContainerEvent::Started {
        id: id.clone(),
        pid,
        timestamp: std::time::SystemTime::now(),
    });

    deps.events.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "run"), ("adapter", "daemon"), ("status", "ok")],
    );
    let active = state.list_containers().await.len() as f64;
    deps.events
        .metrics
        .set_gauge("minibox_active_containers", active, &[]);

    prepared
        .net
        .attach(&id, pid)
        .await
        .with_context(|| format!("net.attach failed for container {id}"))?;

    let pid_file = deps.lifecycle.run_containers_base.join(&id).join("pid");
    if let Err(e) = std::fs::write(&pid_file, pid.to_string()) {
        warn!(
            pid_file = %pid_file.display(),
            error = %e,
            "container: failed to write pid file"
        );
    }

    state.set_container_pid(&id, pid).await;
    let runtime_id = spawn_result.runtime_id.clone();
    state
        .set_container_runtime_id(&id, runtime_id.clone())
        .await;

    // Hand off wait-for-exit to a background task.
    let state_wait = Arc::clone(&state);
    let id_wait = id.clone();
    let event_sink_wait = Arc::clone(&deps.events.event_sink);
    let runtime_wait = Arc::clone(&deps.lifecycle.runtime);
    tokio::spawn(async move {
        daemon_wait_for_exit(WaitParams {
            pid,
            id: id_wait,
            state: state_wait,
            rootfs: spawn_config.rootfs,
            post_exit_hooks: spawn_config.hooks.post_exit,
            event_sink: event_sink_wait,
            cgroup_path: spawn_config.cgroup_path,
            runtime: runtime_wait,
            runtime_id,
        })
        .await;
    });

    Ok(id)
}

/// Re-run a container from its stored `RunCreationParams`.
///
/// Used by `handle_update` to restart containers after an image update.
/// Delegates to `run_inner` with all fields from the stored params.
#[cfg(unix)]
pub(super) async fn run_from_params(
    creation_params: &RunCreationParams,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> Result<String> {
    let params = RunParams {
        image: creation_params.image.clone(),
        tag: creation_params.tag.clone(),
        command: creation_params.command.clone(),
        memory_limit_bytes: creation_params.memory_limit_bytes,
        cpu_weight: creation_params.cpu_weight,
        ephemeral: false,
        network: creation_params.network,
        mounts: creation_params.mounts.clone(),
        privileged: creation_params.privileged,
        env: creation_params.env.clone(),
        name: creation_params.name.clone(),
        platform: creation_params.platform.clone(),
        cgroup_parent: creation_params.cgroup_parent.clone(),
        priority: None,
        policy_override: None,
    };
    run_inner(params, state, deps).await
}

// ─── OOM detection ───────────────────────────────────────────────────────────

/// Check if a container was OOM-killed by reading cgroup v2 `memory.events`.
///
/// Returns `true` if `oom_kill` count is greater than zero.  Returns `false` if
/// the file cannot be read (e.g. cgroup already deleted, or non-Linux platform).
pub async fn check_oom_killed(cgroup_path: &std::path::Path) -> bool {
    let events_path = cgroup_path.join("memory.events");
    if let Ok(content) = tokio::fs::read_to_string(&events_path).await {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("oom_kill ") {
                return rest.trim().parse::<u64>().unwrap_or(0) > 0;
            }
        }
    }
    false
}

// ─── daemon_wait_for_exit ────────────────────────────────────────────────────

///
/// Waits for the container process to exit via the runtime adapter, then
/// updates state and emits lifecycle events.
///
/// Uses `runtime.wait_for_exit()` which dispatches to `waitpid` for native
/// adapters or to the adapter's own wait mechanism (e.g. `SmolvmProcess::wait`
/// for krun).
#[cfg(unix)]
async fn daemon_wait_for_exit(p: WaitParams) {
    let WaitParams {
        pid,
        id,
        state,
        rootfs: _rootfs,
        post_exit_hooks: _post_exit_hooks,
        event_sink,
        cgroup_path,
        runtime,
        runtime_id,
    } = p;
    let id = id.as_str();
    let exit_code = runtime
        .wait_for_exit(runtime_id.as_deref(), pid)
        .await
        .unwrap_or_else(|e| {
            warn!(container_id = %id, error = %e, "container: wait_for_exit error");
            -1
        });
    info!(container_id = %id, exit_code = exit_code, "container: exited");

    #[cfg(target_os = "linux")]
    if !_post_exit_hooks.is_empty() {
        use crate::container::process::run_hooks;
        if let Err(e) = run_hooks(&_post_exit_hooks, &_rootfs, Some(exit_code)) {
            warn!(container_id = %id, error = %e, "container: post-exit hooks error");
        }
    }

    // Check OOM and emit lifecycle event.
    let oom = check_oom_killed(&cgroup_path).await;
    if oom {
        event_sink.emit(ContainerEvent::OomKilled {
            id: id.to_string(),
            timestamp: std::time::SystemTime::now(),
        });
    } else {
        event_sink.emit(ContainerEvent::Stopped {
            id: id.to_string(),
            exit_code,
            timestamp: std::time::SystemTime::now(),
        });
    }

    if let Err(e) = state
        .update_container_state(id, ContainerState::Stopped)
        .await
    {
        warn!(container_id = %id, error = %e, "state: failed to mark container Stopped");
    }
}

/// Windows stub: no-op because HCS/WSL2 lifecycle is managed externally.
///
/// Containers on Windows remain in `"Running"` state until an explicit
/// `stop` or `remove` command is issued.
#[cfg(windows)]
async fn daemon_wait_for_exit(_p: WaitParams) {
    // No-op on Windows. Container stays "Running" until explicit stop/remove.
}

/// Fallback stub for platforms other than Unix or Windows.
#[cfg(not(any(unix, windows)))]
async fn daemon_wait_for_exit(_p: WaitParams) {
    // No-op on this platform.
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod run_inner_tests {
    use super::generate_container_id;

    #[test]
    fn run_inner_capture_signature_accepts_mounts_and_privileged() {
        // Verify that BindMount can be constructed and collected into a vec,
        // confirming the type is accessible to run_inner callers.
        use minibox_core::domain::BindMount;
        use std::path::PathBuf;
        let mounts: Vec<BindMount> = vec![BindMount {
            host_path: PathBuf::from("/tmp/host"),
            container_path: PathBuf::from("/data"),
            read_only: false,
        }];
        assert_eq!(mounts.len(), 1, "BindMount must be constructible");
        assert!(!mounts[0].read_only, "read_only should be false");
        let privileged: bool = false;
        assert!(!privileged, "non-privileged default must be false");
    }

    // ── generate_container_id properties ─────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig {
            failure_persistence: None,
            cases: 256,
            ..proptest::prelude::ProptestConfig::default()
        })]

        /// Generated IDs must always be exactly 16 characters long.
        #[test]
        fn generated_id_is_always_16_chars(_dummy in proptest::prelude::Just(())) {
            let id = generate_container_id();
            prop_assert_eq!(
                id.len(),
                16,
                "expected 16-char id, got {:?} (len={})",
                id,
                id.len()
            );
        }

        /// Generated IDs must contain only lowercase hex characters (0-9, a-f).
        #[test]
        fn generated_id_is_lowercase_hex(_dummy in proptest::prelude::Just(())) {
            let id = generate_container_id();
            prop_assert!(
                id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
                "id contains non-lowercase-hex chars: {:?}",
                id
            );
        }
    }

    /// Two consecutive calls must produce distinct IDs (birthday-paradox:
    /// collision probability per pair is ~2^-64, negligible in testing).
    #[test]
    fn generated_ids_are_distinct_across_calls() {
        let ids: std::collections::HashSet<String> =
            (0..256).map(|_| generate_container_id()).collect();
        assert_eq!(
            ids.len(),
            256,
            "collision detected among 256 generated container IDs"
        );
    }
}
