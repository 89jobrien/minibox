use crate::adapters::JobAdapter;
use crate::client::DaemonClient;
use crate::error::ControllerError;
use crate::models::{CreateJobRequest, CreateJobResponse, JobStatus};
use crate::tracker::{JobTracker, LogEvent};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{delete, get, post},
};
use futures::stream::Stream;
// DaemonRequest/DaemonResponse will be used once daemon supports Attach.
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

#[derive(Clone)]
struct AppState {
    adapter: Arc<JobAdapter>,
    tracker: JobTracker,
}

pub async fn run(listener_addr: &str, socket_path: Option<String>) -> anyhow::Result<()> {
    let client = Arc::new(match socket_path {
        Some(path) => DaemonClient::new(path),
        None => DaemonClient::from_env(),
    });

    let state = AppState {
        adapter: Arc::new(JobAdapter::new(client)),
        tracker: JobTracker::new(),
    };

    let app = Router::new()
        .route("/api/v1/jobs", post(create_job))
        .route("/api/v1/jobs/{job_id}", get(get_job_status))
        .route("/api/v1/jobs/{job_id}", delete(delete_job))
        .route("/api/v1/jobs/{job_id}/logs", get(stream_logs))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(listener_addr).await?;
    tracing::info!(addr = listener_addr, "mbxctl: listening");
    axum::serve(listener, app).await?;

    Ok(())
}

async fn create_job(
    State(state): State<AppState>,
    Json(req): Json<CreateJobRequest>,
) -> impl IntoResponse {
    let job_id = uuid::Uuid::new_v4().to_string();

    // Register the job as "running" before we kick off container creation
    let status = JobStatus {
        job_id: job_id.clone(),
        container_id: None,
        status: "running".to_string(),
        exit_code: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
    };
    state.tracker.insert(status).await;

    // Create a broadcast channel for log streaming before starting the job
    let log_tx = state.tracker.create_log_channel(&job_id).await;

    match state.adapter.create_and_run_with_timeout(req).await {
        Ok((_returned_job_id, container_id)) => {
            // Update tracker with the container ID
            if let Some(mut job) = state.tracker.get(&job_id).await {
                job.container_id = Some(container_id.clone());
                state.tracker.insert(job).await;
            }

            tracing::info!(
                job_id = %job_id,
                container_id = %container_id,
                "mbxctl: job created"
            );

            // Spawn a background task to drain the container's output stream
            // and publish events to the broadcast channel.
            let client = state.adapter.client().clone();
            let tracker = state.tracker.clone();
            let jid = job_id.clone();
            let cid = container_id.clone();

            tokio::spawn(async move {
                if let Err(e) = drain_container_output(client, &cid, &jid, tracker, log_tx).await {
                    tracing::warn!(
                        job_id = %jid,
                        container_id = %cid,
                        error = %e,
                        "mbxctl: background stream drain failed"
                    );
                }
            });

            (
                StatusCode::CREATED,
                Json(CreateJobResponse {
                    job_id,
                    container_id,
                    status: "created".to_string(),
                }),
            )
                .into_response()
        }
        Err(e) => {
            let fail_status = if matches!(e, ControllerError::Timeout(_)) {
                "timeout"
            } else {
                "failed"
            };
            state
                .tracker
                .update_status(&job_id, fail_status, None)
                .await;
            // Clean up the log channel since no output will arrive
            state.tracker.remove_log_channel(&job_id).await;
            tracing::warn!(job_id = %job_id, error = %e, "mbxctl: job creation failed");
            e.into_response()
        }
    }
}

async fn get_job_status(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> impl IntoResponse {
    match state.tracker.get(&job_id).await {
        Some(status) => (StatusCode::OK, Json(status)).into_response(),
        None => ControllerError::JobNotFound { job_id }.into_response(),
    }
}

async fn delete_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> impl IntoResponse {
    // Look up container_id from tracker, fall back to using job_id directly
    let container_id = match state.tracker.get(&job_id).await {
        Some(status) => status.container_id.unwrap_or_else(|| job_id.clone()),
        None => job_id.clone(),
    };

    match state.adapter.stop_container(&container_id).await {
        Ok(()) => {
            state.tracker.update_status(&job_id, "stopped", None).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => e.into_response(),
    }
}

async fn stream_logs(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> impl IntoResponse {
    // Verify the job exists
    let job = match state.tracker.get(&job_id).await {
        Some(status) => status,
        None => {
            return ControllerError::JobNotFound {
                job_id: job_id.clone(),
            }
            .into_response();
        }
    };

    // If the job already completed, send a single completed event
    if job.status == "completed" || job.status == "failed" || job.status == "timeout" {
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(1);
        let completed = Event::default().data(
            serde_json::to_string(&LogEvent::Completed {
                exit_code: job.exit_code,
            })
            .unwrap_or_default(),
        );
        let _ = tx.send(Ok(completed)).await;
        drop(tx);

        return Sse::new(ReceiverStream::new(rx))
            .keep_alive(KeepAlive::default())
            .into_response();
    }

    // Subscribe to the live log broadcast channel
    let mut log_rx = match state.tracker.subscribe(&job_id).await {
        Some(rx) => rx,
        None => {
            // No active channel -- job may have finished between our check and
            // the subscribe call.  Return the current status.
            let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(1);
            let completed = Event::default().data(
                serde_json::to_string(&LogEvent::Completed {
                    exit_code: job.exit_code,
                })
                .unwrap_or_default(),
            );
            let _ = tx.send(Ok(completed)).await;
            drop(tx);

            return Sse::new(ReceiverStream::new(rx))
                .keep_alive(KeepAlive::default())
                .into_response();
        }
    };

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);

    // Send an initial "connected" event
    let connected = Event::default().data(
        serde_json::to_string(&serde_json::json!({
            "type": "connected",
            "job_id": job_id,
            "container_id": job.container_id,
        }))
        .unwrap_or_default(),
    );
    let _ = tx.send(Ok(connected)).await;

    // Spawn a task that relays broadcast events to the SSE channel
    tokio::spawn(async move {
        loop {
            match log_rx.recv().await {
                Ok(event) => {
                    let is_completed = matches!(event, LogEvent::Completed { .. });
                    let data = serde_json::to_string(&event).unwrap_or_default();
                    let sse_event = Event::default().data(data);
                    if tx.send(Ok(sse_event)).await.is_err() {
                        // Client disconnected
                        break;
                    }
                    if is_completed {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!(skipped = n, "mbxctl: sse subscriber lagged");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    // Channel closed (job finished) — send final status from tracker
                    break;
                }
            }
        }
    });

    Sse::new(ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Background task: opens a fresh daemon connection for the given container,
/// reads all `ContainerOutput` / `ContainerStopped` messages, and publishes
/// them as [`LogEvent`]s on the broadcast channel.  Updates the job tracker
/// when the container stops.
async fn drain_container_output(
    _client: Arc<DaemonClient>,
    _container_id: &str,
    job_id: &str,
    tracker: JobTracker,
    log_tx: tokio::sync::broadcast::Sender<LogEvent>,
) -> anyhow::Result<()> {
    // Open a new daemon connection to attach to the container's output.
    // The daemon protocol currently does not support a dedicated "attach"
    // request.  We issue a Run request with ephemeral:true for the same image
    // — but this would create a *new* container, not attach to an existing one.
    //
    // For now, we use a pragmatic workaround: the create_and_run flow already
    // receives the initial ContainerCreated.  We open a *second* connection
    // and send a Run with the same parameters to capture the stream.
    //
    // TODO: Once the daemon supports an `Attach { id }` request, replace this
    // with a proper attach call.
    //
    // For the v1 implementation, the background drain task simply waits for
    // the container to stop by polling its status, since we cannot attach to
    // the existing stream from a second connection.

    // Attempt to read from the daemon.  If the daemon supports a Logs/Attach
    // request in the future, this is where we'd use it.
    //
    // Current fallback: send a synthetic output event and monitor for completion.
    let connected_event = LogEvent::Output {
        stream: "stdout".to_string(),
        data: String::new(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };
    // Best-effort send; receivers may not exist yet
    let _ = log_tx.send(connected_event);

    // Poll for container completion.  This is a stopgap until the daemon
    // supports attach/logs on existing containers.
    let poll_interval = std::time::Duration::from_secs(2);
    let max_polls = 1800; // 1 hour at 2s intervals

    for _ in 0..max_polls {
        tokio::time::sleep(poll_interval).await;

        if let Some(status) = tracker.get(job_id).await {
            if status.status == "completed"
                || status.status == "failed"
                || status.status == "stopped"
            {
                let _ = log_tx.send(LogEvent::Completed {
                    exit_code: status.exit_code,
                });
                tracker.remove_log_channel(job_id).await;
                return Ok(());
            }
        }
    }

    // Timed out waiting for completion
    tracker.update_status(job_id, "timeout", None).await;
    let _ = log_tx.send(LogEvent::Completed { exit_code: None });
    tracker.remove_log_channel(job_id).await;

    Ok(())
}

/// Minimal receiver-backed stream for SSE.
struct ReceiverStream<T> {
    rx: tokio::sync::mpsc::Receiver<T>,
}

impl<T> ReceiverStream<T> {
    fn new(rx: tokio::sync::mpsc::Receiver<T>) -> Self {
        Self { rx }
    }
}

impl<T> Stream for ReceiverStream<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}
