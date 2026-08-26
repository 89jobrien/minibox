//! Pipeline handler: run a crux pipeline inside an ephemeral container.
// Handler signatures require >5 parameters by design (DI pattern). See rustqual.toml.
#![allow(clippy::too_many_arguments)]

use minibox_core::domain::BindMount;
use minibox_core::protocol::DaemonResponse;
use minibox_core::trace::TraceFilter;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::daemon::state::DaemonState;

use super::run::{RunParams, handle_run};
use super::{HandlerDependencies, PolicyOverride, send_error};

/// Bundled user-supplied parameters for a pipeline run request.
pub struct PipelineParams {
    /// Host path to the crux pipeline definition.
    pub pipeline_path: String,
    /// Optional JSON input supplied to the pipeline.
    pub input: Option<serde_json::Value>,
    /// Optional container image override.
    pub image: Option<String>,
    /// Optional crux budget configuration.
    pub budget: Option<serde_json::Value>,
    /// Environment variables passed to the pipeline container.
    pub env: Vec<(String, String)>,
}

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
/// Build the container env list for a pipeline run: inherit caller pairs,
/// inject `CRUX_PLUGIN_PATH`, and optionally `CRUX_BUDGET_JSON` / `CRUX_INPUT_JSON`.
fn build_pipeline_env(
    caller_env: Vec<(String, String)>,
    budget: Option<&serde_json::Value>,
    input: Option<&serde_json::Value>,
) -> Vec<String> {
    let mut env: Vec<String> = caller_env
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    env.push("CRUX_PLUGIN_PATH=/usr/local/bin/minibox-crux-plugin".to_string());
    if let Some(b) = budget
        && let Ok(s) = serde_json::to_string(b)
    {
        env.push(format!("CRUX_BUDGET_JSON={s}"));
    }
    if let Some(inp) = input
        && let Ok(s) = serde_json::to_string(inp)
    {
        env.push(format!("CRUX_INPUT_JSON={s}"));
    }
    env
}

/// Build the bind mount that maps the host pipeline file into the container.
fn build_pipeline_mount(host_pipeline: std::path::PathBuf) -> BindMount {
    BindMount {
        host_path: host_pipeline,
        container_path: std::path::PathBuf::from("/pipeline.crux"),
        read_only: true,
    }
}

/// Validates and launches a pipeline container, then reports its trace.
pub async fn handle_pipeline(
    params: PipelineParams,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let PipelineParams {
        pipeline_path,
        input,
        image,
        budget,
        env,
    } = params;
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

        let pipeline_mount = build_pipeline_mount(host_pipeline);
        let container_env = build_pipeline_env(env, budget.as_ref(), input.as_ref());

        // Bridge channel: collect all streaming responses from handle_run internally.
        const PIPELINE_CHANNEL_CAPACITY: usize = 64;
        let (inner_tx, mut inner_rx) =
            tokio::sync::mpsc::channel::<DaemonResponse>(PIPELINE_CHANNEL_CAPACITY);

        let pipeline_state = Arc::clone(&state);
        let pipeline_deps_clone = Arc::clone(&deps);

        // Spawn handle_run in the background; we drain inner_rx below.
        tokio::spawn(async move {
            let params = RunParams {
                image: image_ref,
                tag: None,
                command: vec![
                    "crux".to_string(),
                    "run".to_string(),
                    "/pipeline.crux".to_string(),
                    "--output".to_string(),
                    "/trace.json".to_string(),
                ],
                memory_limit_bytes: None,
                cpu_weight: None,
                ephemeral: true,
                network: None,
                mounts: vec![pipeline_mount],
                privileged: false,
                env: container_env,
                name: None,
                platform: None,
                cgroup_parent: None,
                priority: None,
                policy_override: Some(PolicyOverride {
                    allow_bind_mounts: Some(true),
                    ..Default::default()
                }),
            };
            handle_run(params, pipeline_state, pipeline_deps_clone, inner_tx).await;
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
        let trace_path = deps
            .lifecycle
            .containers_base
            .join(&container_id)
            .join("upper")
            .join("trace.json");

        let trace_path_clone = trace_path.clone();
        let container_id_clone = container_id.clone();
        let trace = tokio::task::spawn_blocking(move || {
            read_trace_file(&trace_path_clone, &container_id_clone)
        })
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "read_trace_file task panicked");
            serde_json::json!({"steps": []})
        });

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

/// Read and parse `trace.json` from the overlay upper dir.
///
/// Returns the parsed JSON value, or a synthetic empty trace if the file is
/// absent or unparseable.
fn read_trace_file(trace_path: &std::path::Path, container_id: &str) -> serde_json::Value {
    if trace_path.exists() {
        match std::fs::read_to_string(trace_path) {
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
    }
}

/// List pipeline runs from the trace store.
///
/// No per-request auth check: the Unix socket is already gated by
/// `SO_PEERCRED` (UID==0) at connection accept time, so all callers
/// reaching this handler are pre-authorized.
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
///
/// No per-request auth check — see [`handle_list_pipelines`] rationale.
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
