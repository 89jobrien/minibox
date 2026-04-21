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

    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(listener_addr).await?;
    tracing::info!(addr = listener_addr, "miniboxctl: listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C signal handler");
    tracing::info!("miniboxctl: shutdown signal received");
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
                "miniboxctl: job created"
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
                        "miniboxctl: background stream drain failed"
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
            tracing::warn!(job_id = %job_id, error = %e, "miniboxctl: job creation failed");
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
                    tracing::debug!(skipped = n, "miniboxctl: sse subscriber lagged");
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

/// Build the axum router wired to the given state.
///
/// Extracted so tests can construct the router without binding a real TCP listener.
fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/jobs", post(create_job))
        .route("/api/v1/jobs/{job_id}", get(get_job_status))
        .route("/api/v1/jobs/{job_id}", delete(delete_job))
        .route("/api/v1/jobs/{job_id}/logs", get(stream_logs))
        // TODO(roadmap/mcp): add a thin raw-command/attach surface here so
        // miniboxctl can back the planned minibox MCP server instead of forcing all
        // agent workflows through the job abstraction.
        .layer(DefaultBodyLimit::max(1024 * 1024)) // 1 MB
        .with_state(state)
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
                    "miniboxctl: daemon stream closed before ContainerStopped"
                );
                tracker.update_status(job_id, "failed", None).await;
                let _ = log_tx.send(LogEvent::Completed { exit_code: None });
                tracker.remove_log_channel(job_id).await;
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// Build a test router backed by a stub client (no real socket).
    fn test_router() -> Router {
        let client = Arc::new(DaemonClient::new("/nonexistent/test.sock".to_string()));
        let state = AppState {
            adapter: Arc::new(JobAdapter::new(client)),
            tracker: JobTracker::new(),
        };
        build_router(state)
    }

    /// Helper: collect response body bytes and deserialize as JSON.
    async fn body_json(body: Body) -> serde_json::Value {
        let bytes = body.collect().await.expect("collect body").to_bytes();
        serde_json::from_slice(&bytes).expect("parse JSON body")
    }

    // ── ControllerError HTTP status mapping ──────────────────────────────────

    /// `JobNotFound` must map to 404, not 500.
    /// Regression: if the wrong status code is returned, callers cannot
    /// distinguish "job missing" from "server broken".
    #[test]
    fn error_job_not_found_is_404() {
        let err = ControllerError::JobNotFound {
            job_id: "abc".to_string(),
        };
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// `DaemonUnavailable` must map to 503, not 500.
    /// Regression: callers use this to decide whether to retry.
    #[test]
    fn error_daemon_unavailable_is_503() {
        let err = ControllerError::DaemonUnavailable("socket gone".to_string());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    /// `Timeout` must map to 408 Request Timeout, not 500.
    #[test]
    fn error_timeout_is_408() {
        let err = ControllerError::Timeout("job exceeded timeout".to_string());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::REQUEST_TIMEOUT);
    }

    /// `ContainerFailed` and `Internal` both map to 500.
    #[test]
    fn error_container_failed_is_500() {
        let err = ControllerError::ContainerFailed {
            message: "oom".to_string(),
        };
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ── GET /api/v1/jobs/{job_id} ─────────────────────────────────────────────

    /// Unknown job IDs must return 404 with a JSON error body, not panic.
    /// Regression: prevents path-parameter injection from crashing the server.
    #[tokio::test]
    async fn get_unknown_job_returns_404() {
        let router = test_router();

        let req = Request::builder()
            .method(Method::GET)
            .uri("/api/v1/jobs/does-not-exist")
            .body(Body::empty())
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let json = body_json(resp.into_body()).await;
        assert!(
            json.get("error").is_some(),
            "response body must contain an 'error' field: {json}"
        );
    }

    /// A job ID containing path-traversal characters (`../`) must still return
    /// 404 (axum normalises the path), not a 200 or a panic.
    #[tokio::test]
    async fn get_job_path_traversal_returns_404_or_bad_request() {
        let router = test_router();

        // axum will reject or normalise — either way must not be 200/500
        let req = Request::builder()
            .method(Method::GET)
            .uri("/api/v1/jobs/..%2F..%2Fetc%2Fpasswd")
            .body(Body::empty())
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");
        assert_ne!(
            resp.status(),
            StatusCode::OK,
            "path traversal must not succeed"
        );
        assert_ne!(
            resp.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "path traversal must not cause 500"
        );
    }

    // ── POST /api/v1/jobs ─────────────────────────────────────────────────────

    /// Malformed JSON must return a 4xx client error (axum extractor rejection),
    /// not a 500 or a panic.
    /// Regression: ensures serde validation is wired correctly.
    /// Note: axum returns 400 Bad Request for syntactically invalid JSON and
    /// 422 Unprocessable Entity for structurally valid JSON with missing fields.
    #[tokio::test]
    async fn post_malformed_json_returns_4xx() {
        let router = test_router();

        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/jobs")
            .header("content-type", "application/json")
            .body(Body::from("{not valid json"))
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");
        let status = resp.status().as_u16();
        assert!(
            (400..500).contains(&status),
            "malformed JSON must return 4xx, got {status}"
        );
        assert_ne!(
            resp.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "malformed JSON must not cause 500"
        );
    }

    /// Missing required fields (image, command) must return 422.
    #[tokio::test]
    async fn post_missing_required_fields_returns_422() {
        let router = test_router();

        // `command` field is required (Vec<String> with no default)
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/jobs")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"image": "alpine"}"#))
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// A request body larger than 1 MB must be rejected with 413.
    /// Regression: the `DefaultBodyLimit` layer must stay in place.
    #[tokio::test]
    async fn post_oversized_body_returns_413() {
        let router = test_router();

        // 1.1 MB of filler (just over the 1 MB limit)
        let big_body = "x".repeat(1024 * 1024 + 1024);
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/jobs")
            .header("content-type", "application/json")
            .body(Body::from(big_body))
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    // ── DELETE /api/v1/jobs/{job_id} ──────────────────────────────────────────

    /// Deleting an unknown job must not panic — it attempts a daemon stop which
    /// will fail (no socket), returning an error response, not a 500 from a
    /// missing-job check.
    #[tokio::test]
    async fn delete_unknown_job_does_not_panic() {
        let router = test_router();

        let req = Request::builder()
            .method(Method::DELETE)
            .uri("/api/v1/jobs/ghost-job-id")
            .body(Body::empty())
            .expect("build request");

        // The handler falls back to using the job_id as the container_id and
        // attempts a stop on the (nonexistent) daemon. It must return an error
        // response, not panic.
        let resp = router.oneshot(req).await.expect("oneshot");
        assert_ne!(
            resp.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "missing job must not produce an unhandled 500: got {}",
            resp.status()
        );
    }
}
