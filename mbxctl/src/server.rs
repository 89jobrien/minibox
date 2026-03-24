use crate::adapters::JobAdapter;
use crate::client::DaemonClient;
use crate::models::{CreateJobRequest, CreateJobResponse};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};
use std::sync::Arc;

type AppState = Arc<JobAdapter>;

pub async fn run(listener_addr: &str, socket_path: Option<String>) -> anyhow::Result<()> {
    let client = Arc::new(match socket_path {
        Some(path) => DaemonClient::new(path),
        None => DaemonClient::from_env(),
    });

    let adapter = Arc::new(JobAdapter::new(client));

    let app = Router::new()
        .route("/api/v1/jobs", post(create_job))
        .route("/api/v1/jobs/{job_id}", get(get_job_status))
        .route("/api/v1/jobs/{job_id}", delete(delete_job))
        .route("/api/v1/jobs/{job_id}/logs", get(stream_logs))
        .with_state(adapter);

    let listener = tokio::net::TcpListener::bind(listener_addr).await?;
    tracing::info!(addr = listener_addr, "mbxctl: listening");
    axum::serve(listener, app).await?;

    Ok(())
}

async fn create_job(
    State(adapter): State<AppState>,
    Json(req): Json<CreateJobRequest>,
) -> impl IntoResponse {
    match adapter.create_and_run(req).await {
        Ok((job_id, container_id)) => (
            StatusCode::CREATED,
            Json(CreateJobResponse {
                job_id,
                container_id,
                status: "created".to_string(),
            }),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}

async fn get_job_status(Path(_job_id): Path<String>) -> impl IntoResponse {
    // TODO: Implement in Task 8
    StatusCode::NOT_IMPLEMENTED
}

async fn delete_job(
    State(adapter): State<AppState>,
    Path(job_id): Path<String>,
) -> impl IntoResponse {
    match adapter.stop_container(&job_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn stream_logs(Path(_job_id): Path<String>) -> impl IntoResponse {
    // TODO: Implement SSE streaming in Task 8
    StatusCode::NOT_IMPLEMENTED
}
