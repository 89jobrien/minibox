//! Container lifecycle handlers: pause, resume, remove, list.

use minibox_core::domain::DomainError;
use minibox_core::events::{ContainerEvent, EventSink};
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::daemon::state::{ContainerState, DaemonState};

use super::super::network_lifecycle::NetworkLifecycle;
use super::HandlerDependencies;

// ─── Pause / Resume ─────────────────────────────────────────────────────────

/// Look up a container by `id`, verify its state matches `expected_state`, and
/// return the `cgroup.freeze` path. Returns `Err(DaemonResponse::Error)` if
/// the container is missing or in an unexpected state.
async fn freeze_path_for(
    id: &str,
    state: &DaemonState,
    expected_state: ContainerState,
    verb: &str,
) -> Result<std::path::PathBuf, DaemonResponse> {
    let record = match state.get_container(id).await {
        Some(r) => r,
        None => {
            return Err(DaemonResponse::Error {
                message: format!("container {id} not found"),
            });
        }
    };
    if record.info.state != expected_state.as_str() {
        return Err(DaemonResponse::Error {
            message: format!(
                "container {id} is not {verb} (state: {})",
                record.info.state
            ),
        });
    }
    Ok(record.cgroup_path.join("cgroup.freeze"))
}

/// Freeze a running container by writing `1` to its `cgroup.freeze` file.
///
/// Returns `DaemonResponse::ContainerPaused` on success, `DaemonResponse::Error`
/// if the container is not found, not running, or the cgroup write fails.
pub async fn handle_pause(
    id: String,
    state: Arc<DaemonState>,
    event_sink: Arc<dyn EventSink>,
) -> DaemonResponse {
    let freeze_path = match freeze_path_for(&id, &state, ContainerState::Running, "running").await {
        Ok(p) => p,
        Err(resp) => return resp,
    };
    if let Err(e) = tokio::fs::write(&freeze_path, "1\n").await {
        return DaemonResponse::Error {
            message: format!("pause failed: {e}"),
        };
    }
    if let Err(e) = state
        .update_container_state(&id, ContainerState::Paused)
        .await
    {
        warn!(container_id = %id, error = %e, "state: failed to mark paused");
    }
    info!(container_id = %id, "container: paused");
    event_sink.emit(ContainerEvent::Paused {
        id: id.clone(),
        timestamp: std::time::SystemTime::now(),
    });
    DaemonResponse::ContainerPaused { id }
}

/// Unfreeze a paused container by writing `0` to its `cgroup.freeze` file.
///
/// Returns `DaemonResponse::ContainerResumed` on success, `DaemonResponse::Error`
/// if the container is not found, not paused, or the cgroup write fails.
pub async fn handle_resume(
    id: String,
    state: Arc<DaemonState>,
    event_sink: Arc<dyn EventSink>,
) -> DaemonResponse {
    let freeze_path = match freeze_path_for(&id, &state, ContainerState::Paused, "paused").await {
        Ok(p) => p,
        Err(resp) => return resp,
    };
    if let Err(e) = tokio::fs::write(&freeze_path, "0\n").await {
        return DaemonResponse::Error {
            message: format!("resume failed: {e}"),
        };
    }
    if let Err(e) = state
        .update_container_state(&id, ContainerState::Running)
        .await
    {
        warn!(container_id = %id, error = %e, "state: failed to mark running after resume");
    }
    info!(container_id = %id, "container: resumed");
    event_sink.emit(ContainerEvent::Resumed {
        id: id.clone(),
        timestamp: std::time::SystemTime::now(),
    });
    DaemonResponse::ContainerResumed { id }
}

// ─── Remove ─────────────────────────────────────────────────────────────────

/// Clean up a stopped container: unmount overlay, delete dirs, remove cgroup.
pub async fn handle_remove(
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

    let result = remove_inner(&id, &state, &deps).await;
    let status = if result.is_ok() { "ok" } else { "error" };
    deps.events.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "remove"), ("adapter", "daemon"), ("status", status)],
    );

    match result {
        Ok(()) => {
            let active = state.list_containers().await.len() as f64;
            deps.events
                .metrics
                .set_gauge("minibox_active_containers", active, &[]);
            DaemonResponse::Success {
                message: format!("container {id} removed"),
            }
        }
        Err(e) => {
            error!("handle_remove error: {e:#}");
            DaemonResponse::Error {
                message: format!("{e:#}"),
            }
        }
    }
}

/// Core remove logic: unmount overlay, delete runtime state dir, clean up
/// cgroup, and deregister the container from the daemon state.
///
/// Returns an error if the container does not exist or is still `"Running"`.
/// Cleanup steps (overlay unmount, cgroup removal) are best-effort: failures
/// are logged as warnings but do not abort the removal.
async fn remove_inner(
    id: &str,
    state: &Arc<DaemonState>,
    deps: &Arc<HandlerDependencies>,
) -> anyhow::Result<()> {
    let record = state
        .get_container(id)
        .await
        .ok_or_else(|| DomainError::ContainerNotFound { id: id.to_string() })?;

    if record.info.state == "Running" || record.info.state == "Paused" {
        return Err(DomainError::ContainerNotStopped {
            id: id.to_string(),
            state: record.info.state.clone(),
        }
        .into());
    }

    // Unmount overlay (using injected filesystem trait).
    let container_dir = deps.lifecycle.containers_base.join(id);
    if container_dir.exists() {
        // SECURITY: assert the path is under the expected base to prevent
        // accidental recursive deletion outside the containers directory.
        if !container_dir.starts_with(&deps.lifecycle.containers_base) {
            warn!(
                path = %container_dir.display(),
                base = %deps.lifecycle.containers_base.display(),
                "remove: container_dir escapes base directory, skipping cleanup"
            );
        } else if let Err(e) = deps.lifecycle.filesystem.cleanup(&container_dir) {
            warn!("cleanup_mounts for {id}: {e}");
        }
    }

    // Remove runtime state directory.
    let run_dir = deps.lifecycle.run_containers_base.join(id);
    if run_dir.exists() {
        // SECURITY: assert the path is under the expected base to prevent
        // accidental recursive deletion outside the run directory.
        if run_dir.starts_with(&deps.lifecycle.run_containers_base) {
            std::fs::remove_dir_all(&run_dir).ok();
        } else {
            warn!(
                path = %run_dir.display(),
                base = %deps.lifecycle.run_containers_base.display(),
                "remove: run_dir escapes base directory, skipping removal"
            );
        }
    }

    // Cleanup cgroup (using injected resource limiter trait).
    if let Err(e) = deps.lifecycle.resource_limiter.cleanup(id) {
        warn!("cleanup cgroup for {id}: {e}");
    }

    // ── Network cleanup ────────────────────────────────────────────────
    NetworkLifecycle::new(deps.lifecycle.network_provider.clone())
        .cleanup(id)
        .await;

    state.remove_container(id).await;
    Ok(())
}

// ─── List ───────────────────────────────────────────────────────────────────

/// Return all known containers.
pub async fn handle_list(state: Arc<DaemonState>) -> DaemonResponse {
    let containers = state.list_containers().await;
    DaemonResponse::ContainerList { containers }
}
