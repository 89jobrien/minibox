use crate::adapters::JobAdapter;
use crate::client::{DaemonClient, ResponseStream};
use crate::error::ControllerError;
use crate::models::{CreateJobRequest, CreateJobResponse, JobStatus};
use crate::tracker::{JobTracker, LogEvent};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{delete, get, post},
};
use minibox_core::protocol::{DaemonResponse, OutputStreamKind};
// DaemonRequest/DaemonResponse will be used once daemon supports Attach.
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::ReceiverStream;

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
        .layer(DefaultBodyLimit::max(1024 * 1024)) // 1 MB
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(listener_addr).await?;
    tracing::info!(addr = listener_addr, "mbxctl: listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C signal handler");
    tracing::info!("mbxctl: shutdown signal received");
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
        Ok((container_id, response_stream)) => {
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

            // Drain the live stream in the background, forwarding output to
            // SSE subscribers via the broadcast channel.
            let tracker = state.tracker.clone();
            let jid = job_id.clone();
            let cid = container_id.clone();

            tokio::spawn(async move {
                if let Err(e) =
                    drain_container_output(response_stream, &cid, &jid, tracker, log_tx).await
                {
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

/// Background task: drains `ContainerOutput` / `ContainerStopped` messages
/// from the live daemon stream and publishes them as [`LogEvent`]s on the
/// broadcast channel.  Updates the job tracker when the container stops.
async fn drain_container_output(
    mut stream: ResponseStream,
    container_id: &str,
    job_id: &str,
    tracker: JobTracker,
    log_tx: tokio::sync::broadcast::Sender<LogEvent>,
) -> anyhow::Result<()> {
    loop {
        match stream.next().await? {
            Some(DaemonResponse::ContainerOutput { stream: kind, data }) => {
                let stream_name = match kind {
                    OutputStreamKind::Stdout => "stdout",
                    OutputStreamKind::Stderr => "stderr",
                };
                let _ = log_tx.send(LogEvent::Output {
                    stream: stream_name.to_string(),
                    data,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                });
            }
            Some(DaemonResponse::ContainerStopped { exit_code }) => {
                tracker
                    .update_status(job_id, "completed", Some(exit_code))
                    .await;
                let _ = log_tx.send(LogEvent::Completed {
                    exit_code: Some(exit_code),
                });
                tracker.remove_log_channel(job_id).await;
                return Ok(());
            }
            Some(DaemonResponse::Error { message }) => {
                tracker.update_status(job_id, "failed", None).await;
                let _ = log_tx.send(LogEvent::Completed { exit_code: None });
                tracker.remove_log_channel(job_id).await;
                anyhow::bail!("container error: {}", message);
            }
            Some(_) => continue,
            None => {
                // Stream closed without ContainerStopped — treat as failure
                tracing::warn!(
                    job_id = %job_id,
                    container_id = %container_id,
                    "mbxctl: daemon stream closed before ContainerStopped"
                );
                tracker.update_status(job_id, "failed", None).await;
                let _ = log_tx.send(LogEvent::Completed { exit_code: None });
                tracker.remove_log_channel(job_id).await;
                return Ok(());
            }
        }
    }
}
