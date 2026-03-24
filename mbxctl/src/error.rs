use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ControllerError {
    #[error("daemon unavailable: {0}")]
    DaemonUnavailable(String),

    #[error("job not found: {job_id}")]
    JobNotFound { job_id: String },

    #[error("container failed: {message}")]
    ContainerFailed { message: String },

    #[error("timeout: {0}")]
    Timeout(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for ControllerError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ControllerError::DaemonUnavailable(msg) => {
                (StatusCode::SERVICE_UNAVAILABLE, msg.clone())
            }
            ControllerError::JobNotFound { job_id } => {
                (StatusCode::NOT_FOUND, format!("job not found: {job_id}"))
            }
            ControllerError::ContainerFailed { message } => {
                (StatusCode::INTERNAL_SERVER_ERROR, message.clone())
            }
            ControllerError::Timeout(msg) => (StatusCode::REQUEST_TIMEOUT, msg.clone()),
            ControllerError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}
