//! VM state snapshot handlers: save, restore, list.

use chrono::Utc;
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;

use crate::daemon::state::DaemonState;

use super::HandlerDependencies;

/// Save a VM state snapshot for a container.
pub async fn handle_save_snapshot(
    id: String,
    name: Option<String>,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    let snap_name = name.unwrap_or_else(|| Utc::now().format("%Y%m%d-%H%M%S").to_string());

    let data_dir = &deps.lifecycle.containers_base;
    let snap_dir = data_dir
        .parent()
        .unwrap_or(data_dir)
        .join("snapshots")
        .join(&id);
    if let Err(e) = std::fs::create_dir_all(&snap_dir) {
        return DaemonResponse::Error {
            message: format!("failed to create snapshot dir: {e}"),
        };
    }

    let snap_path = snap_dir.join(format!("{snap_name}.snap"));
    match deps.checkpoint.save_snapshot(&id, &snap_path) {
        Ok(info) => DaemonResponse::SnapshotSaved { info },
        Err(e) => DaemonResponse::Error {
            message: format!("save_snapshot: {e}"),
        },
    }
}

/// Restore a VM state snapshot for a container.
pub async fn handle_restore_snapshot(
    id: String,
    name: String,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    let data_dir = &deps.lifecycle.containers_base;
    let snap_path = data_dir
        .parent()
        .unwrap_or(data_dir)
        .join("snapshots")
        .join(&id)
        .join(format!("{name}.snap"));

    match deps.checkpoint.restore_snapshot(&id, &snap_path) {
        Ok(()) => DaemonResponse::SnapshotRestored { id, name },
        Err(e) => DaemonResponse::Error {
            message: format!("restore_snapshot: {e}"),
        },
    }
}

/// List available snapshots for a container.
pub async fn handle_list_snapshots(id: String, deps: Arc<HandlerDependencies>) -> DaemonResponse {
    match deps.checkpoint.list_snapshots(&id) {
        Ok(snapshots) => DaemonResponse::SnapshotList { id, snapshots },
        Err(e) => DaemonResponse::Error {
            message: format!("list_snapshots: {e}"),
        },
    }
}
