use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bytes::BytesMut;
use futures::StreamExt as _;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::AppState;
use crate::domain::{ExecConfig, RuntimeError};

/// Build a Docker 8-byte multiplexed log frame.
fn frame_log_chunk(stream_type: u8, data: &[u8]) -> bytes::Bytes {
    let len = data.len() as u32;
    let mut frame = BytesMut::with_capacity(8 + data.len());
    frame.extend_from_slice(&[stream_type, 0, 0, 0]);
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(data);
    frame.freeze()
}

/// POST /containers/{id}/exec — create an exec instance.
pub async fn create(
    State(state): State<AppState>,
    Path(container_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let cmd: Vec<String> = body["Cmd"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let env: Vec<String> = body["Env"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let attach_stdout = body["AttachStdout"].as_bool().unwrap_or(true);
    let attach_stderr = body["AttachStderr"].as_bool().unwrap_or(true);

    let config = ExecConfig {
        cmd,
        env,
        attach_stdout,
        attach_stderr,
    };

    match state.runtime.exec_create(&container_id, config).await {
        Ok(exec_id) => (StatusCode::CREATED, Json(json!({"Id": exec_id}))).into_response(),
        Err(RuntimeError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"message": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"message": e.to_string()})),
        )
            .into_response(),
    }
}

/// POST /exec/{id}/start — run the exec command and stream multiplexed output.
pub async fn start(
    State(state): State<AppState>,
    Path(exec_id): Path<String>,
    Json(_body): Json<Value>,
) -> Response {
    let (tx, rx) = mpsc::channel(64);
    let runtime = state.runtime.clone();
    let exec_id_clone = exec_id.clone();

    tokio::spawn(async move {
        let _ = runtime.exec_start(&exec_id_clone, tx).await;
    });

    let stream = ReceiverStream::new(rx).map(|chunk| {
        let framed = frame_log_chunk(chunk.stream, &chunk.data);
        Ok::<_, std::convert::Infallible>(framed)
    });

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/vnd.docker.raw-stream")
        .body(Body::from_stream(stream))
        .unwrap()
}

/// GET /exec/{id}/json — inspect exec result.
pub async fn inspect(State(state): State<AppState>, Path(exec_id): Path<String>) -> Response {
    match state.runtime.exec_inspect(&exec_id).await {
        Ok(details) => {
            let body = json!({
                "ID": details.id,
                "Running": details.running,
                "ExitCode": details.exit_code,
                "ProcessConfig": {
                    "tty": false,
                    "entrypoint": "",
                    "arguments": []
                },
                "OpenStdin": false,
                "OpenStdout": true,
                "OpenStderr": true,
                "CanRemove": false,
                "ContainerID": ""
            });
            Json(body).into_response()
        }
        Err(RuntimeError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"message": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"message": e.to_string()})),
        )
            .into_response(),
    }
}
