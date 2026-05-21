//! Pipeline handler: run a crux pipeline inside an ephemeral container.

use minibox_core::domain::BindMount;
use minibox_core::protocol::DaemonResponse;
use minibox_core::trace::TraceFilter;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::daemon::state::DaemonState;

use super::run::handle_run;
use super::{HandlerDependencies, send_error};

/// Run a crux pipeline inside an ephemeral container.
///
/// Higher-level than `handle_run`: pulls image, creates container with the
/// pipeline file bind-mounted at `/pipeline.crux`, streams `ContainerOutput`
/// to the client, then after the container exits reads `/trace.json` from the
/// overlay upper dir and emits [`DaemonResponse::PipelineComplete`].
///
/// # Protocol sequence
///
/// ```text
/// Client  ──RunPipeline──►  Daemon
/// Client  ◄──ContainerCreated──  (container ID)
/// Client  ◄──ContainerOutput──   (zero or more stdout/stderr chunks)
/// Client  ◄──PipelineComplete──  (trace + exit_code; terminal)
/// ```
///
/// On macOS / non-Unix platforms the streaming run path is unavailable;
/// `handle_pipeline` returns an `Error` response immediately on those builds.
///
/// # Trace file
///
/// After the container exits the handler looks for `<containers_base>/<id>/upper/trace.json`.
/// If the file is present it is parsed as JSON and included in `PipelineComplete.trace`.
/// If absent or unparseable, a synthetic empty trace `{"steps":[]}` is used —
/// the pipeline still completes successfully (the exit code determines success).
#[allow(clippy::too_many_arguments)]
pub async fn handle_pipeline(
    pipeline_path: String,
    input: Option<serde_json::Value>,
    image: Option<String>,
    budget: Option<serde_json::Value>,
    env: Vec<(String, String)>,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    #[cfg(not(unix))]
    {
        let _ = (pipeline_path, input, image, budget, env, state, deps);
        send_error(
            &tx,
            "handle_pipeline",
            "RunPipeline is only supported on Unix platforms".to_string(),
        )
        .await;
        return;
    }

    #[cfg(unix)]
    {
        let image_ref = image.unwrap_or_else(|| "crux-runtime:latest".to_string());

        // Validate pipeline path is absolute so the bind-mount is unambiguous.
        let host_pipeline = std::path::PathBuf::from(&pipeline_path);
        if !host_pipeline.is_absolute() {
            send_error(
                &tx,
                "handle_pipeline",
                format!("pipeline_path must be absolute, got: {pipeline_path:?}"),
            )
            .await;
            return;
        }

        // Build the bind mount: pipeline file → /pipeline.crux (read-only).
        let pipeline_mount = BindMount {
            host_path: host_pipeline,
            container_path: std::path::PathBuf::from("/pipeline.crux"),
            read_only: true,
        };

        // Build env list: inherit caller env, add CRUX_PLUGIN_PATH and optional budget.
        let mut container_env: Vec<String> =
            env.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
        container_env.push("CRUX_PLUGIN_PATH=/usr/local/bin/minibox-crux-plugin".to_string());
        if let Some(ref b) = budget
            && let Ok(s) = serde_json::to_string(b)
        {
            container_env.push(format!("CRUX_BUDGET_JSON={s}"));
        }
        if let Some(inp) = &input
            && let Ok(s) = serde_json::to_string(inp)
        {
            container_env.push(format!("CRUX_INPUT_JSON={s}"));
        }

        // Clone deps and override policy to permit bind mounts for this
        // internal pipeline run.  Pipeline requests originate from the daemon
        // (not from an end user), so the bind-mount policy exception is safe.
        let mut pipeline_deps = (*deps).clone();
        pipeline_deps.policy.allow_bind_mounts = true;
        let pipeline_deps = Arc::new(pipeline_deps);

        // Bridge channel: collect all streaming responses from handle_run internally.
        let (inner_tx, mut inner_rx) = tokio::sync::mpsc::channel::<DaemonResponse>(64);

        let pipeline_state = Arc::clone(&state);
        let pipeline_deps_clone = Arc::clone(&pipeline_deps);

        // Spawn handle_run in the background; we drain inner_rx below.
        tokio::spawn(async move {
            handle_run(
                image_ref,
                None,
                vec![
                    "crux".to_string(),
                    "run".to_string(),
                    "/pipeline.crux".to_string(),
                    "--output".to_string(),
                    "/trace.json".to_string(),
                ],
                None,
                None,
                true, // ephemeral: stream output
                None,
                vec![pipeline_mount],
                false,
                container_env,
                None,
                None,
                pipeline_state,
                pipeline_deps_clone,
                inner_tx,
            )
            .await;
        });

        // Drain the inner channel, collecting the container ID and exit code.
        let mut container_id = String::new();
        let mut exit_code = 0i32;

        loop {
            match inner_rx.recv().await {
                None => break,
                Some(DaemonResponse::ContainerCreated { id }) => {
                    container_id = id;
                    // Do not forward ContainerCreated — pipeline clients receive
                    // PipelineComplete instead.
                }
                Some(DaemonResponse::ContainerOutput { stream, data }) => {
                    // Forward output chunks to the client in real time.
                    if tx
                        .send(DaemonResponse::ContainerOutput { stream, data })
                        .await
                        .is_err()
                    {
                        warn!("handle_pipeline: client disconnected during ContainerOutput");
                        return;
                    }
                }
                Some(DaemonResponse::ContainerStopped { exit_code: ec }) => {
                    exit_code = ec;
                    break;
                }
                Some(DaemonResponse::Error { message }) => {
                    // Container failed to start or run — propagate as error.
                    send_error(&tx, "handle_pipeline", message).await;
                    return;
                }
                Some(other) => {
                    debug!(
                        response = ?other,
                        "handle_pipeline: unexpected inner response, ignoring"
                    );
                }
            }
        }

        if container_id.is_empty() {
            send_error(
                &tx,
                "handle_pipeline",
                "pipeline container did not produce a container ID".to_string(),
            )
            .await;
            return;
        }

        // Read trace.json from the overlay upper dir.
        // Path: <containers_base>/<id>/upper/trace.json
        let trace_path = deps
            .lifecycle
            .containers_base
            .join(&container_id)
            .join("upper")
            .join("trace.json");

        let trace = if trace_path.exists() {
            match std::fs::read_to_string(&trace_path) {
                Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(v) => {
                        info!(
                            container_id = %container_id,
                            path = %trace_path.display(),
                            "handle_pipeline: trace loaded"
                        );
                        v
                    }
                    Err(e) => {
                        warn!(
                            container_id = %container_id,
                            path = %trace_path.display(),
                            error = %e,
                            "handle_pipeline: trace file is not valid JSON, using empty trace"
                        );
                        serde_json::json!({"steps": []})
                    }
                },
                Err(e) => {
                    warn!(
                        container_id = %container_id,
                        path = %trace_path.display(),
                        error = %e,
                        "handle_pipeline: failed to read trace file, using empty trace"
                    );
                    serde_json::json!({"steps": []})
                }
            }
        } else {
            debug!(
                container_id = %container_id,
                path = %trace_path.display(),
                "handle_pipeline: no trace file found, using empty trace"
            );
            serde_json::json!({"steps": []})
        };

        info!(
            container_id = %container_id,
            exit_code,
            "handle_pipeline: pipeline complete"
        );

        // Persist the trace before notifying the client so that a client that
        // immediately queries `mbx pipeline show <id>` sees the record.
        {
            let store = state.trace_store.clone();
            let id_for_store = container_id.clone();
            let pipeline_for_store = pipeline_path.clone();
            let trace_for_store = trace.clone();
            if let Err(e) = store.store(
                &id_for_store,
                &pipeline_for_store,
                &trace_for_store,
                exit_code,
            ) {
                warn!(
                    container_id = %container_id,
                    error = %e,
                    "handle_pipeline: failed to store trace (non-fatal)"
                );
            }
        }

        if tx
            .send(DaemonResponse::PipelineComplete {
                trace,
                container_id,
                exit_code,
            })
            .await
            .is_err()
        {
            warn!("handle_pipeline: client disconnected before PipelineComplete could be sent");
        }
    }
}

/// List pipeline runs from the trace store.
pub async fn handle_list_pipelines(
    limit: Option<usize>,
    pipeline: Option<String>,
    state: Arc<DaemonState>,
) -> DaemonResponse {
    let filter = TraceFilter {
        limit,
        pipeline,
        ..Default::default()
    };
    match state.trace_store.list(&filter) {
        Ok(pipelines) => DaemonResponse::PipelineList { pipelines },
        Err(e) => DaemonResponse::Error {
            message: format!("failed to list pipelines: {e}"),
        },
    }
}

/// Show details of a specific pipeline run.
pub async fn handle_show_pipeline(id: String, state: Arc<DaemonState>) -> DaemonResponse {
    match state.trace_store.load(&id) {
        Ok(Some(trace)) => DaemonResponse::PipelineDetail { id, trace },
        Ok(None) => DaemonResponse::Error {
            message: format!("pipeline run not found: {id}"),
        },
        Err(e) => DaemonResponse::Error {
            message: format!("failed to load pipeline run {id}: {e}"),
        },
    }
}
