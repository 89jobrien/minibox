//! Execution manifest inspection and verification handlers.

use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::warn;

use crate::daemon::state::DaemonState;

use super::{HandlerDependencies, send_error};

/// Retrieve the execution manifest for a container.
// qual:allow(complexity) reason: "manifest retrieval with fallback paths"
pub async fn handle_get_manifest(
    id: String,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let record = match state.get_container(&id).await {
        Some(r) => r,
        None => {
            send_error(
                &tx,
                "handle_get_manifest",
                format!("container not found: {id}"),
            )
            .await;
            return;
        }
    };

    let manifest_path = match record.manifest_path {
        Some(p) => p,
        None => deps
            .lifecycle
            .containers_base
            .join(&id)
            .join("execution-manifest.json"),
    };

    let content = match std::fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(e) => {
            send_error(
                &tx,
                "handle_get_manifest",
                format!(
                    "failed to read manifest at {}: {e}",
                    manifest_path.display()
                ),
            )
            .await;
            return;
        }
    };

    // Deserialize as typed struct to validate schema integrity before
    // returning to the client.
    let manifest: minibox_core::domain::ExecutionManifest = match serde_json::from_str(&content) {
        Ok(m) => m,
        Err(e) => {
            send_error(
                &tx,
                "handle_get_manifest",
                format!("failed to parse manifest JSON: {e}"),
            )
            .await;
            return;
        }
    };

    let manifest_value = match serde_json::to_value(&manifest) {
        Ok(v) => v,
        Err(e) => {
            send_error(
                &tx,
                "handle_get_manifest",
                format!("failed to re-serialize manifest: {e}"),
            )
            .await;
            return;
        }
    };

    if tx
        .send(DaemonResponse::Manifest {
            manifest: manifest_value,
        })
        .await
        .is_err()
    {
        warn!(container_id = %id, "handle_get_manifest: client disconnected");
    }
}

/// Verify a container's execution manifest against an execution policy.
// qual:allow(complexity) reason: "manifest verification with policy evaluation"
pub async fn handle_verify_manifest(
    id: String,
    policy_json: String,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    use minibox_core::domain::{ExecutionManifest, ExecutionPolicy, PolicyDecision};

    let record = match state.get_container(&id).await {
        Some(r) => r,
        None => {
            send_error(
                &tx,
                "handle_verify_manifest",
                format!("container not found: {id}"),
            )
            .await;
            return;
        }
    };

    let manifest_path = match record.manifest_path {
        Some(p) => p,
        None => deps
            .lifecycle
            .containers_base
            .join(&id)
            .join("execution-manifest.json"),
    };

    let content = match std::fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(e) => {
            send_error(
                &tx,
                "handle_verify_manifest",
                format!("failed to read manifest: {e}"),
            )
            .await;
            return;
        }
    };

    let manifest: ExecutionManifest = match serde_json::from_str(&content) {
        Ok(m) => m,
        Err(e) => {
            send_error(
                &tx,
                "handle_verify_manifest",
                format!("failed to parse manifest: {e}"),
            )
            .await;
            return;
        }
    };

    let policy: ExecutionPolicy = match serde_json::from_str(&policy_json) {
        Ok(p) => p,
        Err(e) => {
            send_error(
                &tx,
                "handle_verify_manifest",
                format!("failed to parse policy: {e}"),
            )
            .await;
            return;
        }
    };

    let decision = policy.evaluate(&manifest);
    let (allowed, reason) = match decision {
        PolicyDecision::Allow => (true, None),
        PolicyDecision::Deny(reason) => (false, Some(reason)),
    };

    if tx
        .send(DaemonResponse::VerifyResult { allowed, reason })
        .await
        .is_err()
    {
        warn!(container_id = %id, "handle_verify_manifest: client disconnected");
    }
}
