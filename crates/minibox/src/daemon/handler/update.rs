//! Image update handler: re-pull cached images, optionally restart containers.
// TODO(#178): add regression test for handle_update restart ContainerRecord gap

use minibox_core::image::reference::ImageRef;
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::daemon::state::DaemonState;

use super::run::run_from_params;
use super::stop::stop_inner;
use super::{HandlerDependencies, send_error};

/// Bundled user-supplied parameters for an image update request.
pub struct UpdateParams {
    /// Explicit image references to update.
    pub images: Vec<String>,
    /// Whether to update every cached image.
    pub all: bool,
    /// Whether to update images referenced by containers.
    pub containers: bool,
    /// Whether affected containers should be restarted.
    pub restart: bool,
}

/// Resolve the list of image refs to update based on `all`, `containers`, or
/// explicit `images` list.
async fn resolve_update_targets(
    images: Vec<String>,
    all: bool,
    containers: bool,
    state: &Arc<DaemonState>,
    deps: &Arc<HandlerDependencies>,
) -> std::result::Result<Vec<String>, String> {
    if all {
        deps.image
            .image_store
            .list_all_images()
            .await
            .map_err(|e| format!("failed to list images: {e:#}"))
    } else if containers {
        let containers_list = state.list_containers().await;
        let mut seen = std::collections::HashSet::new();
        let mut refs = Vec::new();
        for info in containers_list {
            let record = state.get_container(&info.id).await;
            if let Some(source_ref) = record.and_then(|r| r.source_image_ref)
                && seen.insert(source_ref.clone())
            {
                refs.push(source_ref);
            }
        }
        Ok(refs)
    } else {
        Ok(images)
    }
}

/// Re-pull cached images to pick up newer versions.
///
/// Sends a non-terminal [`DaemonResponse::UpdateProgress`] for each image
/// processed, then a terminal [`DaemonResponse::Success`] with a summary.
///
/// # Image resolution order
///
/// 1. If `all` is `true`: every image returned by [`minibox_core::image::ImageStore::list_all_images`].
/// 2. If `containers` is `true`: deduplicated `source_image_ref` values from all
///    container records held in `state`.
/// 3. Otherwise: the explicit `images` list.
///
/// When `restart` is `true`, Running or Paused containers whose source image
/// was updated are stopped and re-run from their stored `creation_params`.
// qual:allow(iosp) reason: "handler orchestration — resolve images, pull, restart"
pub async fn handle_update(
    p: UpdateParams,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let UpdateParams {
        images,
        all,
        containers,
        restart,
    } = p;
    // ── Step 1: resolve the list of image refs to update ─────────────────────
    let target_refs = match resolve_update_targets(images, all, containers, &state, &deps).await {
        Ok(refs) => refs,
        Err(msg) => {
            send_error(&tx, "handle_update", msg).await;
            return;
        }
    };

    let total = target_refs.len();
    let mut updated: usize = 0;

    // ── Step 2: pull each image, send UpdateProgress per image ────────────────
    for ref_str in &target_refs {
        let image_ref = match ImageRef::parse(ref_str) {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    image = %ref_str,
                    error = %e,
                    "handle_update: invalid image reference, skipping"
                );
                let status = format!("error: {e}");
                if tx
                    .send(DaemonResponse::UpdateProgress {
                        image: ref_str.clone(),
                        status,
                    })
                    .await
                    .is_err()
                {
                    warn!(
                        image = %ref_str,
                        "handle_update: client disconnected during UpdateProgress"
                    );
                    return;
                }
                continue;
            }
        };

        let registry = deps.image.registry_router.route(&image_ref);
        let status = match registry.pull_image(&image_ref).await {
            Ok(_) => {
                info!(
                    image = %ref_str,
                    "handle_update: image refreshed"
                );
                updated += 1;
                "updated".to_string()
            }
            Err(e) => {
                warn!(
                    image = %ref_str,
                    error = %e,
                    "handle_update: pull failed"
                );
                format!("error: {e:#}")
            }
        };

        if tx
            .send(DaemonResponse::UpdateProgress {
                image: ref_str.clone(),
                status,
            })
            .await
            .is_err()
        {
            warn!(
                image = %ref_str,
                "handle_update: client disconnected during UpdateProgress"
            );
            return;
        }
    }

    // ── Step 3: restart containers using updated images (restart = true) ──────
    //
    // For each Running or Paused container whose source image was just updated:
    // 1. Stop the container
    // 2. Re-run it from stored creation_params so it picks up the new layers
    //
    // Containers without creation_params are stop-only (legacy records).
    //
    // stop_inner is unix-only so this entire block is cfg-gated.
    #[cfg(unix)]
    let (stopped, restarted): (usize, usize) = if restart {
        let target_set: std::collections::HashSet<&str> =
            target_refs.iter().map(String::as_str).collect();

        let candidate_ids: Vec<String> = state
            .list_containers()
            .await
            .into_iter()
            .filter(|info| info.state == "Running" || info.state == "Paused")
            .map(|info| info.id)
            .collect();

        let mut stop_count = 0usize;
        let mut restart_count = 0usize;
        for id in candidate_ids {
            let record = state.get_container(&id).await;
            let (image_ref, creation_params) = match &record {
                Some(r) => (r.source_image_ref.as_deref(), r.creation_params.clone()),
                None => (None, None),
            };
            if !image_ref.is_some_and(|r| target_set.contains(r)) {
                continue;
            }

            info!(
                container_id = %id,
                "handle_update: stopping container for image update (restart=true)"
            );
            match stop_inner(&id, &state).await {
                Ok(()) => {
                    stop_count += 1;
                    info!(container_id = %id, "handle_update: container stopped");
                }
                Err(e) => {
                    warn!(
                        container_id = %id,
                        error = %e,
                        "handle_update: failed to stop container — continuing"
                    );
                    continue;
                }
            }

            // Re-run from stored creation params if available.
            if let Some(params) = creation_params {
                match run_from_params(&params, Arc::clone(&state), Arc::clone(&deps)).await {
                    Ok(new_id) => {
                        restart_count += 1;
                        info!(
                            old_id = %id,
                            new_id = %new_id,
                            "handle_update: container restarted with updated image"
                        );
                        if tx
                            .send(DaemonResponse::UpdateProgress {
                                image: params.image.clone(),
                                status: format!("restarted {id} as {new_id}"),
                            })
                            .await
                            .is_err()
                        {
                            warn!("handle_update: client disconnected during restart progress");
                            return;
                        }
                    }
                    Err(e) => {
                        warn!(
                            container_id = %id,
                            error = %e,
                            "handle_update: failed to restart container — stopped but not re-run"
                        );
                    }
                }
            } else {
                warn!(
                    container_id = %id,
                    "handle_update: no creation_params — stopped but cannot restart"
                );
            }
        }
        (stop_count, restart_count)
    } else {
        (0, 0)
    };

    #[cfg(not(unix))]
    let (stopped, restarted): (usize, usize) = {
        if restart {
            warn!("handle_update: restart not supported on this platform");
        }
        (0, 0)
    };

    // ── Step 4: terminal Success ──────────────────────────────────────────────
    let message = if restart && stopped > 0 {
        format!(
            "updated {updated}/{total} images; stopped {stopped}, restarted {restarted} container(s)"
        )
    } else {
        format!("updated {updated}/{total} images")
    };
    info!(updated, total, "handle_update: complete");
    if tx.send(DaemonResponse::Success { message }).await.is_err() {
        warn!("handle_update: client disconnected before Success could be sent");
    }
}
