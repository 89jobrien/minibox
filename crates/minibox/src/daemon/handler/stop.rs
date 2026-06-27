//! Stop handler and platform-specific `stop_inner` implementations.

use anyhow::Result;
use minibox_core::domain::DomainError;
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

use crate::daemon::state::{ContainerState, DaemonState};

use super::super::network_lifecycle::NetworkLifecycle;
use super::HandlerDependencies;

/// Record the container op counter and return the status label.
fn record_stop_op(deps: &HandlerDependencies, ok: bool) -> &'static str {
    let status = if ok { "ok" } else { "error" };
    deps.events.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "stop"), ("adapter", "daemon"), ("status", status)],
    );
    status
}

/// Send SIGTERM to a container, then SIGKILL after 10 seconds if needed.
pub async fn handle_stop(
    name_or_id: String,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    let id = match state.resolve_id(&name_or_id).await {
        Some(id) => id,
        None => {
            return DaemonResponse::Error {
                message: format!("container not found: {name_or_id}"),
            };
        }
    };

    // ── Network cleanup ────────────────────────────────────────────────
    NetworkLifecycle::new(deps.lifecycle.network_provider.clone())
        .cleanup(&id)
        .await;

    let result = stop_inner(&id, &state).await;
    record_stop_op(&deps, result.is_ok());

    match result {
        Ok(()) => {
            let active = state.list_containers().await.len() as f64;
            deps.events
                .metrics
                .set_gauge("minibox_active_containers", active, &[]);
            DaemonResponse::Success {
                message: format!("container {id} stopped"),
            }
        }
        Err(e) => {
            error!("handle_stop error: {e:#}");
            DaemonResponse::Error {
                message: format!("{e:#}"),
            }
        }
    }
}

/// Unix implementation: send SIGTERM, poll for exit for up to 2 s, then
/// SIGKILL if the process is still alive.  Updates state to `"Stopped"` on
/// completion regardless of how the process exited.
#[cfg(unix)]
pub(super) async fn stop_inner(id: &str, state: &Arc<DaemonState>) -> Result<()> {
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
    // Signal the entire process group so descendants (e.g. `sleep` spawned
    // by `/bin/sh -c …`) receive SIGTERM directly.  child_init calls setsid()
    // before execve, making the container init a new process group leader;
    // negating its host PID addresses that group.  We fall back to the
    // individual PID if the group signal returns ESRCH (process already gone).
    let pgid = Pid::from_raw(-(pid as i32));

    info!(
        container_id = %id,
        pid = pid,
        "container: sending SIGTERM to process group"
    );
    if kill(pgid, Signal::SIGTERM).is_err() {
        kill(nix_pid, Signal::SIGTERM).ok();
    }

    // Wait up to 2 s for the process to exit gracefully.  In practice,
    // PID 1 in a PID namespace silently ignores SIGTERM (kernel-enforced),
    // so busybox `sh -c …` containers will never respond.  We keep a short
    // window for containers that do install a handler, then fall through to
    // SIGKILL promptly.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        const STOP_POLL_INTERVAL_MS: u64 = 250;
        tokio::time::sleep(Duration::from_millis(STOP_POLL_INTERVAL_MS)).await;
        if kill(nix_pid, None).is_err() {
            // ESRCH — process is gone.
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            warn!(
                container_id = %id,
                pid = pid,
                "container: did not exit after SIGTERM, sending SIGKILL"
            );
            kill(pgid, Signal::SIGKILL).ok();
            kill(nix_pid, Signal::SIGKILL).ok();
            break;
        }
    }

    if let Err(e) = state
        .update_container_state(id, ContainerState::Stopped)
        .await
    {
        warn!(container_id = %id, error = %e, "state: failed to mark container Stopped");
    }
    Ok(())
}

/// Windows stub: stop is not yet implemented.
///
/// Container stop must go through the HCS or WSL2 adapter stop path.
/// This stub ensures the binary compiles on Windows and returns a clear error.
#[cfg(windows)]
pub(super) async fn stop_inner(id: &str, _state: &Arc<DaemonState>) -> Result<()> {
    anyhow::bail!(
        "handle_stop not yet implemented on Windows for container {id} \
         — use the HCS/WSL2 adapter stop path"
    )
}

/// Fallback stub for platforms other than Unix or Windows.
#[cfg(not(any(unix, windows)))]
pub(super) async fn stop_inner(id: &str, _state: &Arc<DaemonState>) -> Result<()> {
    anyhow::bail!("handle_stop not supported on this platform for container {id}")
}
