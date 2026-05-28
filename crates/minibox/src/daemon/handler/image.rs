//! Image handlers: pull, load, push, commit, build, prune, remove, list.

use anyhow::Result;
use minibox_core::events::EventSink;
use minibox_core::image::reference::ImageRef;
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, instrument, warn};

use crate::daemon::state::DaemonState;

use super::{HandlerDependencies, send_error};

// ─── Platform registry resolution ────────────────────────────────────────────

/// Apply a per-request platform override to whichever registry the router selected.
///
/// Downcasts the routed registry to its concrete type and reconstructs it with
/// the requested [`TargetPlatform`].  Returns `None` when `platform` is absent
/// (the caller should use the router's result directly).
///
/// # Errors
///
/// Returns an error if `platform` cannot be parsed, or if the adapter cannot
/// be reconstructed (e.g. TLS init failure).
pub(super) fn resolve_platform_registry(
    platform: &Option<String>,
    image_ref: &minibox_core::image::reference::ImageRef,
    deps: &HandlerDependencies,
) -> Result<Option<Box<dyn minibox_core::domain::ImageRegistry>>> {
    let Some(p) = platform else {
        return Ok(None);
    };

    let tp = minibox_core::image::manifest::TargetPlatform::parse(p)?;
    info!(platform = %p, "using per-request platform override");

    // Route first so we know which registry type owns this image reference,
    // then reconstruct that adapter with the platform override applied.
    let routed = deps.image.registry_router.route(image_ref);

    if routed.as_any().is::<crate::adapters::GhcrRegistry>() {
        let registry =
            crate::adapters::GhcrRegistry::with_platform(Arc::clone(&deps.image.image_store), tp)?;
        return Ok(Some(Box::new(registry)));
    }

    // Default: treat as Docker Hub (covers `native` adapter and any unknown
    // hostname that the router falls back to its default for).
    let registry =
        crate::adapters::DockerHubRegistry::with_platform(Arc::clone(&deps.image.image_store), tp)?;
    Ok(Some(Box::new(registry)))
}

// ─── Pull ───────────────────────────────────────────────────────────────────

#[instrument(skip(_state, deps), fields(image = %image, tag = ?tag))]
pub async fn handle_pull(
    image: String,
    tag: Option<String>,
    platform: Option<String>,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    // Build full ref string from image + optional tag, then parse into ImageRef.
    let ref_str = match &tag {
        Some(t) => format!("{image}:{t}"),
        None => image.clone(),
    };
    let image_ref = match ImageRef::parse(&ref_str) {
        Ok(r) => r,
        Err(e) => {
            error!("handle_pull: invalid image reference {ref_str:?}: {e}");
            return DaemonResponse::Error {
                message: format!("invalid image reference {ref_str:?}: {e}"),
            };
        }
    };
    let tag = image_ref.tag.clone();

    // When a platform override is requested, reconstruct the routed registry
    // adapter with the requested platform applied. Otherwise use the router's
    // result directly.  The box is held for the lifetime of the pull call.
    let platform_registry = match resolve_platform_registry(&platform, &image_ref, &deps) {
        Ok(r) => r,
        Err(e) => {
            error!("handle_pull: invalid platform: {e}");
            return DaemonResponse::Error {
                message: format!("invalid platform: {e}"),
            };
        }
    };

    let registry: &dyn minibox_core::domain::ImageRegistry = match &platform_registry {
        Some(r) => r.as_ref(),
        None => deps.image.registry_router.route(&image_ref),
    };

    // Pull image (using selected registry trait).
    let start = std::time::Instant::now();
    let (status, response) = match registry.pull_image(&image_ref).await {
        Ok(_metadata) => (
            "ok",
            DaemonResponse::Success {
                message: format!("pulled {image}:{tag}"),
            },
        ),
        Err(e) => {
            error!("handle_pull error: {e:#}");
            (
                "error",
                DaemonResponse::Error {
                    message: format!("{e:#}"),
                },
            )
        }
    };

    deps.events.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "pull"), ("adapter", "daemon"), ("status", status)],
    );
    deps.events.metrics.record_histogram(
        "minibox_container_op_duration_seconds",
        start.elapsed().as_secs_f64(),
        &[("op", "pull"), ("adapter", "daemon")],
    );

    response
}

// ─── Load Image ─────────────────────────────────────────────────────────────

/// Load a local OCI image tarball into the image store.
#[instrument(skip(_state, deps), fields(path = %path, name = %name, tag = %tag))]
pub async fn handle_load_image(
    path: String,
    name: String,
    tag: String,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    let image_path = std::path::Path::new(&path);
    let start = std::time::Instant::now();
    let (status, response) = match deps
        .image
        .image_loader
        .load_image(image_path, &name, &tag)
        .await
    {
        Ok(()) => {
            info!(
                path = %path,
                image = %format!("{name}:{tag}"),
                "load_image: loaded successfully"
            );
            (
                "ok",
                DaemonResponse::ImageLoaded {
                    image: format!("{name}:{tag}"),
                },
            )
        }
        Err(e) => {
            error!(error = %e, "load_image: failed");
            (
                "error",
                DaemonResponse::Error {
                    message: format!("{e:#}"),
                },
            )
        }
    };
    deps.events.metrics.increment_counter(
        "minibox_container_ops_total",
        &[
            ("op", "load_image"),
            ("adapter", "daemon"),
            ("status", status),
        ],
    );
    deps.events.metrics.record_histogram(
        "minibox_container_op_duration_seconds",
        start.elapsed().as_secs_f64(),
        &[("op", "load_image"), ("adapter", "daemon")],
    );
    response
}

// ─── Push ────────────────────────────────────────────────────────────────────

/// Push a locally-stored image to a remote OCI registry.
///
/// Sends zero or more `PushProgress` messages followed by `Success` or `Error`.
// qual:allow(complexity) reason: "push handler: ref parse, adapter dispatch, progress stream"
pub async fn handle_push(
    image_ref_str: String,
    credentials: minibox_core::protocol::PushCredentials,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let start = std::time::Instant::now();
    let Some(ref pusher) = deps.build.image_pusher else {
        deps.events.metrics.increment_counter(
            "minibox_container_ops_total",
            &[("op", "push"), ("adapter", "daemon"), ("status", "error")],
        );
        send_error(
            &tx,
            "handle_push",
            "push not supported on this platform".to_string(),
        )
        .await;
        return;
    };

    let image_ref = match minibox_core::image::reference::ImageRef::parse(&image_ref_str) {
        Ok(r) => r,
        Err(e) => {
            send_error(&tx, "handle_push", format!("invalid image ref: {e}")).await;
            return;
        }
    };

    let creds = match credentials {
        minibox_core::protocol::PushCredentials::Anonymous => {
            minibox_core::domain::RegistryCredentials::Anonymous
        }
        minibox_core::protocol::PushCredentials::Basic { username, password } => {
            minibox_core::domain::RegistryCredentials::Basic { username, password }
        }
        minibox_core::protocol::PushCredentials::Token { token } => {
            minibox_core::domain::RegistryCredentials::Token(token)
        }
    };

    const PUSH_PROGRESS_CHANNEL_CAPACITY: usize = 32;
    let (progress_tx, mut progress_rx) =
        mpsc::channel::<minibox_core::domain::PushProgress>(PUSH_PROGRESS_CHANNEL_CAPACITY);
    let tx2 = tx.clone();
    tokio::spawn(async move {
        while let Some(p) = progress_rx.recv().await {
            let _ = tx2
                .send(DaemonResponse::PushProgress {
                    layer_digest: p.layer_digest,
                    bytes_uploaded: p.bytes_uploaded,
                    total_bytes: p.total_bytes,
                })
                .await;
        }
    });

    match pusher
        .push_image(&image_ref, &creds, Some(progress_tx))
        .await
    {
        Ok(result) => {
            info!(
                image_ref = %image_ref_str,
                digest = %result.digest,
                size_bytes = result.size_bytes,
                "push: completed"
            );
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "push"), ("adapter", "daemon"), ("status", "ok")],
            );
            deps.events.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "push"), ("adapter", "daemon")],
            );
            let _ = tx
                .send(DaemonResponse::Success {
                    message: format!("pushed {} digest:{}", image_ref_str, result.digest),
                })
                .await;
        }
        Err(e) => {
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "push"), ("adapter", "daemon"), ("status", "error")],
            );
            deps.events.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "push"), ("adapter", "daemon")],
            );
            send_error(&tx, "handle_push", e.to_string()).await;
        }
    }
}

// ─── Commit ─────────────────────────────────────────────────────────────────

// qual:allow(complexity) reason: "commit handler: lookup, tar layer, store update"
#[allow(clippy::too_many_arguments)]
pub async fn handle_commit(
    container_id: String,
    target_image: String,
    author: Option<String>,
    message: Option<String>,
    env_overrides: Vec<String>,
    cmd_override: Option<Vec<String>>,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let start = std::time::Instant::now();
    let Some(ref committer) = deps.build.commit_adapter else {
        deps.events.metrics.increment_counter(
            "minibox_container_ops_total",
            &[("op", "commit"), ("adapter", "daemon"), ("status", "error")],
        );
        send_error(
            &tx,
            "handle_commit",
            "commit not supported on this platform".to_string(),
        )
        .await;
        return;
    };

    let cid = match minibox_core::domain::ContainerId::new(container_id.clone()) {
        Ok(id) => id,
        Err(e) => {
            send_error(&tx, "handle_commit", format!("invalid container id: {e}")).await;
            return;
        }
    };

    let config = minibox_core::domain::CommitConfig {
        author,
        message,
        env_overrides,
        cmd_override,
    };

    match committer.commit(&cid, &target_image, &config).await {
        Ok(meta) => {
            info!(
                container_id = %container_id,
                target = %target_image,
                layers = meta.layers.len(),
                "commit: completed"
            );
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "commit"), ("adapter", "daemon"), ("status", "ok")],
            );
            deps.events.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "commit"), ("adapter", "daemon")],
            );
            let _ = tx
                .send(DaemonResponse::Success {
                    message: format!(
                        "committed {} digest:{}",
                        target_image,
                        meta.layers
                            .first()
                            .map(|l| l.digest.as_str())
                            .unwrap_or("unknown")
                    ),
                })
                .await;
        }
        Err(e) => {
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "commit"), ("adapter", "daemon"), ("status", "error")],
            );
            deps.events.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "commit"), ("adapter", "daemon")],
            );
            send_error(&tx, "handle_commit", e.to_string()).await;
        }
    }
}

// ─── Build ──────────────────────────────────────────────────────────────────

/// Build an image from an inline Dockerfile string.
///
/// Streams [`DaemonResponse::BuildOutput`] for each Dockerfile step, then
/// sends exactly one terminal response: [`DaemonResponse::BuildComplete`] on
/// success or [`DaemonResponse::Error`] on failure.
// qual:allow(complexity) reason: "build handler: Dockerfile parse, step execution, progress"
#[allow(clippy::too_many_arguments)]
pub async fn handle_build(
    dockerfile: String,
    context_path: String,
    tag: String,
    build_args: Vec<(String, String)>,
    no_cache: bool,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let start = std::time::Instant::now();
    let Some(ref builder) = deps.build.image_builder else {
        deps.events.metrics.increment_counter(
            "minibox_container_ops_total",
            &[("op", "build"), ("adapter", "daemon"), ("status", "error")],
        );
        send_error(
            &tx,
            "handle_build",
            "build not supported on this platform".to_string(),
        )
        .await;
        return;
    };

    // SECURITY: context_path comes from the protocol request. SO_PEERCRED restricts
    // who can connect (UID 0 only), but not what paths they may name. We canonicalize
    // to resolve symlinks and reject relative paths before touching the filesystem.
    let context_dir = {
        let raw = std::path::PathBuf::from(&context_path);
        if !raw.is_absolute() {
            send_error(
                &tx,
                "handle_build",
                format!("build context_path must be absolute: {context_path:?}"),
            )
            .await;
            return;
        }
        match raw.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                send_error(
                    &tx,
                    "handle_build",
                    format!("build context_path invalid: {e}"),
                )
                .await;
                return;
            }
        }
    };
    let dockerfile_path = context_dir.join("Dockerfile.minibox-build");
    if let Err(e) = tokio::fs::write(&dockerfile_path, &dockerfile).await {
        send_error(&tx, "handle_build", format!("write Dockerfile: {e}")).await;
        return;
    }

    let context = minibox_core::domain::BuildContext {
        directory: context_dir,
        dockerfile: std::path::PathBuf::from("Dockerfile.minibox-build"),
    };
    let config = minibox_core::domain::BuildConfig {
        tag: tag.clone(),
        build_args,
        no_cache,
    };

    const BUILD_PROGRESS_CHANNEL_CAPACITY: usize = 64;
    let (progress_tx, mut progress_rx) =
        mpsc::channel::<minibox_core::domain::BuildProgress>(BUILD_PROGRESS_CHANNEL_CAPACITY);
    let tx2 = tx.clone();
    tokio::spawn(async move {
        while let Some(p) = progress_rx.recv().await {
            let _ = tx2
                .send(DaemonResponse::BuildOutput {
                    step: p.step,
                    total_steps: p.total_steps,
                    message: p.message,
                })
                .await;
        }
    });

    match builder.build_image(&context, &config, progress_tx).await {
        Ok(meta) => {
            info!(
                tag = %tag,
                layers = meta.layers.len(),
                "build: complete"
            );
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "build"), ("adapter", "daemon"), ("status", "ok")],
            );
            deps.events.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "build"), ("adapter", "daemon")],
            );
            let image_id = meta
                .layers
                .first()
                .map(|l| l.digest.clone())
                .unwrap_or_else(|| format!("built:{tag}"));
            let _ = tx
                .send(DaemonResponse::BuildComplete { image_id, tag })
                .await;
        }
        Err(e) => {
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "build"), ("adapter", "daemon"), ("status", "error")],
            );
            deps.events.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "build"), ("adapter", "daemon")],
            );
            send_error(&tx, "handle_build", format!("build failed: {e}")).await;
        }
    }
}

// ─── Prune ──────────────────────────────────────────────────────────────────

/// Remove unused images from the image store.
pub(crate) async fn handle_prune(
    dry_run: bool,
    state: Arc<DaemonState>,
    image_gc: Arc<dyn minibox_core::image::gc::ImageGarbageCollector>,
    event_sink: Arc<dyn EventSink>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let in_use: Vec<String> = state
        .list_containers()
        .await
        .into_iter()
        .filter_map(|c| {
            if c.state == "running" || c.state == "paused" {
                Some(c.image.clone())
            } else {
                None
            }
        })
        .collect();

    match image_gc.prune(dry_run, &in_use).await {
        Ok(report) => {
            let count = report.removed.len();
            let freed = report.freed_bytes;
            event_sink.emit(minibox_core::events::ContainerEvent::ImagePruned {
                count,
                freed_bytes: freed,
                timestamp: std::time::SystemTime::now(),
            });
            let _ = tx
                .send(DaemonResponse::Pruned {
                    removed: report.removed,
                    freed_bytes: freed,
                    dry_run: report.dry_run,
                })
                .await;
        }
        Err(e) => {
            send_error(&tx, "handle_prune", e.to_string()).await;
        }
    }
}

// ─── RemoveImage ─────────────────────────────────────────────────────────────

/// Remove a specific image by reference.
pub(crate) async fn handle_remove_image(
    image_ref: String,
    state: Arc<DaemonState>,
    image_store: Arc<minibox_core::image::ImageStore>,
    event_sink: Arc<dyn EventSink>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let in_use = state
        .list_containers()
        .await
        .into_iter()
        .any(|c| (c.state == "running" || c.state == "paused") && c.image == image_ref);

    if in_use {
        send_error(
            &tx,
            "handle_remove_image",
            format!("image {image_ref} is in use by a running container"),
        )
        .await;
        return;
    }

    let (name, tag) = match image_ref.rsplit_once(':') {
        Some(pair) => pair,
        None => {
            send_error(
                &tx,
                "handle_remove_image",
                format!("invalid image ref: {image_ref}"),
            )
            .await;
            return;
        }
    };

    match image_store.delete_image(name, tag).await {
        Ok(()) => {
            event_sink.emit(minibox_core::events::ContainerEvent::ImageRemoved {
                image: image_ref.clone(),
                timestamp: std::time::SystemTime::now(),
            });
            let _ = tx
                .send(DaemonResponse::Success {
                    message: format!("removed {image_ref}"),
                })
                .await;
        }
        Err(e) => {
            send_error(&tx, "handle_remove_image", e.to_string()).await;
        }
    }
}

/// List all cached images stored in the image store.
pub(crate) async fn handle_list_images(
    image_store: Arc<minibox_core::image::ImageStore>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    match image_store.list_all_images().await {
        Ok(images) => {
            if tx.send(DaemonResponse::ImageList { images }).await.is_err() {
                warn!("handle_list_images: client disconnected before ImageList could be sent");
            }
        }
        Err(e) => {
            send_error(&tx, "handle_list_images", e.to_string()).await;
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod registry_router_tests {
    use crate::adapters::{DockerHubRegistry, GhcrRegistry};
    use minibox_core::adapters::HostnameRegistryRouter;
    use minibox_core::domain::{DynImageRegistry, RegistryRouter};
    use minibox_core::image::ImageStore;
    use minibox_core::image::reference::ImageRef;
    use std::sync::Arc;

    fn make_router(store: &Arc<ImageStore>) -> (HostnameRegistryRouter, *const (), *const ()) {
        let docker: DynImageRegistry = Arc::new(DockerHubRegistry::new(Arc::clone(store)).unwrap());
        let ghcr: DynImageRegistry = Arc::new(GhcrRegistry::new(Arc::clone(store)).unwrap());

        let docker_ptr = Arc::as_ptr(&docker) as *const ();
        let ghcr_ptr = Arc::as_ptr(&ghcr) as *const ();

        let router = HostnameRegistryRouter::new(docker, [("ghcr.io", ghcr)]);
        (router, docker_ptr, ghcr_ptr)
    }

    #[test]
    fn routes_ghcr() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp.path().join("images")).unwrap());
        let (router, _, ghcr_ptr) = make_router(&store);

        let image_ref = ImageRef::parse("ghcr.io/org/minibox-rust-ci:stable").unwrap();
        let selected =
            router.route(&image_ref) as *const dyn minibox_core::domain::ImageRegistry as *const ();

        assert_eq!(selected, ghcr_ptr);
    }

    #[test]
    fn routes_ghcr_case_insensitive() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp.path().join("images")).unwrap());
        let (router, _, ghcr_ptr) = make_router(&store);

        // GHCR.IO (uppercase) must still route to the ghcr adapter
        let image_ref = ImageRef::parse("GHCR.IO/org/image:tag").unwrap();
        let selected =
            router.route(&image_ref) as *const dyn minibox_core::domain::ImageRegistry as *const ();

        assert_eq!(selected, ghcr_ptr);
    }

    #[test]
    fn routes_docker_hub_as_default() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp.path().join("images")).unwrap());
        let (router, docker_ptr, _) = make_router(&store);

        let image_ref = ImageRef::parse("alpine").unwrap();
        let selected =
            router.route(&image_ref) as *const dyn minibox_core::domain::ImageRegistry as *const ();

        assert_eq!(selected, docker_ptr);
    }
}
