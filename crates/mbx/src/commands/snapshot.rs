//! `mbx snapshot` — save, restore, and list VM state snapshots.

use anyhow::Context;
use minibox_core::client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

/// Save a snapshot for a container.
pub async fn execute_save(
    id: String,
    name: Option<String>,
    socket_path: &std::path::Path,
) -> anyhow::Result<()> {
    let request = DaemonRequest::SaveSnapshot {
        id: id.clone(),
        name,
    };

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::SnapshotSaved { info } => {
                println!(
                    "Snapshot saved: {} ({})",
                    info.name, info.container_id
                );
                Ok(())
            }
            DaemonResponse::Error { message } => {
                eprintln!("error: {message}");
                std::process::exit(1);
            }
            other => {
                eprintln!("unexpected response: {other:?}");
                std::process::exit(1);
            }
        }
    } else {
        eprintln!("no response from daemon");
        std::process::exit(1);
    }
}

/// Restore a snapshot for a container.
pub async fn execute_restore(
    id: String,
    name: String,
    socket_path: &std::path::Path,
) -> anyhow::Result<()> {
    let request = DaemonRequest::RestoreSnapshot {
        id: id.clone(),
        name: name.clone(),
    };

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::SnapshotRestored { id, name } => {
                println!("Snapshot restored: {name} for container {id}");
                Ok(())
            }
            DaemonResponse::Error { message } => {
                eprintln!("error: {message}");
                std::process::exit(1);
            }
            other => {
                eprintln!("unexpected response: {other:?}");
                std::process::exit(1);
            }
        }
    } else {
        eprintln!("no response from daemon");
        std::process::exit(1);
    }
}

/// List snapshots for a container.
pub async fn execute_list(
    id: String,
    socket_path: &std::path::Path,
) -> anyhow::Result<()> {
    let request = DaemonRequest::ListSnapshots { id: id.clone() };

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::SnapshotList { id, snapshots } => {
                if snapshots.is_empty() {
                    println!("No snapshots for container {id}");
                } else {
                    println!("Snapshots for container {id}:");
                    for s in &snapshots {
                        println!(
                            "  {} (created: {}, adapter: {}, size: {} bytes)",
                            s.name, s.created_at, s.adapter, s.size_bytes
                        );
                    }
                }
                Ok(())
            }
            DaemonResponse::Error { message } => {
                eprintln!("error: {message}");
                std::process::exit(1);
            }
            other => {
                eprintln!("unexpected response: {other:?}");
                std::process::exit(1);
            }
        }
    } else {
        eprintln!("no response from daemon");
        std::process::exit(1);
    }
}
