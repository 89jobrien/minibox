use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bytes::BytesMut;
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::AppState;
use crate::domain::{CgroupNs, CreateConfig, PortBinding, RuntimeError};

/// Build a Docker 8-byte multiplexed log frame.
fn frame_log_chunk(stream_type: u8, data: &[u8]) -> bytes::Bytes {
    let len = data.len() as u32;
    let mut frame = BytesMut::with_capacity(8 + data.len());
    frame.extend_from_slice(&[stream_type, 0, 0, 0]);
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(data);
    frame.freeze()
}

pub async fn create(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let image = body["Image"].as_str().unwrap_or("").to_string();
    let name = body["name"].as_str().map(str::to_string);

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

    let host_config = &body["HostConfig"];

    let binds: Vec<String> = host_config["Binds"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let privileged = host_config["Privileged"].as_bool().unwrap_or(false);

    let cgroup_ns = match host_config["CgroupnsMode"].as_str() {
        Some("host") => CgroupNs::Host,
        _ => CgroupNs::Private,
    };

    let network_mode = host_config["NetworkMode"].as_str().map(str::to_string);

    // Parse port bindings from PortBindings in HostConfig
    let mut ports = Vec::new();
    if let Some(pb_map) = host_config["PortBindings"].as_object() {
        for (container_port_proto, bindings) in pb_map {
            // container_port_proto looks like "80/tcp"
            let parts: Vec<&str> = container_port_proto.splitn(2, '/').collect();
            let container_port: u16 = parts[0].parse().unwrap_or(0);
            let protocol = parts.get(1).copied().unwrap_or("tcp").to_string();

            if let Some(binding_arr) = bindings.as_array() {
                for binding in binding_arr {
                    let host_port: Option<u16> =
                        binding["HostPort"].as_str().and_then(|p| p.parse().ok());
                    let host_ip = binding["HostIp"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    ports.push(PortBinding {
                        container_port,
                        protocol: protocol.clone(),
                        host_ip,
                        host_port,
                    });
                }
            }
        }
    }

    let labels: std::collections::HashMap<String, String> = body["Labels"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let config = CreateConfig {
        image,
        name,
        cmd,
        env,
        binds,
        ports,
        privileged,
        cgroup_ns,
        network_mode,
        labels,
    };

    match state.runtime.create_container(config).await {
        Ok(id) => (StatusCode::CREATED, Json(json!({"Id": id, "Warnings": []}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"message": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn start(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.runtime.start_container(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
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

pub async fn inspect(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.runtime.inspect_container(&id).await {
        Ok(details) => {
            let body = json!({
                "Id": details.id,
                "Created": details.created,
                "Path": details.config.cmd.first().cloned().unwrap_or_default(),
                "Args": details.config.cmd.iter().skip(1).collect::<Vec<_>>(),
                "State": {
                    "Status": details.status,
                    "Running": details.status == "running",
                    "Paused": false,
                    "Restarting": false,
                    "OOMKilled": false,
                    "Dead": false,
                    "Pid": 0,
                    "ExitCode": details.exit_code.unwrap_or(0),
                    "Error": "",
                    "StartedAt": details.created,
                    "FinishedAt": "0001-01-01T00:00:00Z"
                },
                "Image": details.image,
                "Name": details.name,
                "Config": {
                    "Hostname": &details.id[..12],
                    "Image": details.image,
                    "Cmd": details.config.cmd,
                    "Env": details.config.env,
                    "Labels": details.config.labels
                },
                "HostConfig": {
                    "Binds": details.config.binds,
                    "NetworkMode": details.config.network_mode.unwrap_or_else(|| "default".to_string()),
                    "Privileged": details.config.privileged
                },
                "NetworkSettings": {
                    "Networks": {}
                },
                "Mounts": []
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

#[derive(Deserialize)]
pub struct ListQuery {
    pub all: Option<String>,
}

pub async fn list(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Response {
    let all = q.all.as_deref() == Some("1") || q.all.as_deref() == Some("true");

    match state.runtime.list_containers(all).await {
        Ok(containers) => {
            let result: Vec<Value> = containers
                .into_iter()
                .map(|c| {
                    json!({
                        "Id": c.id,
                        "Names": c.names,
                        "Image": c.image,
                        "ImageID": "",
                        "Command": "",
                        "Created": 0,
                        "Ports": [],
                        "Labels": {},
                        "State": c.state,
                        "Status": c.status,
                        "HostConfig": {"NetworkMode": "default"},
                        "NetworkSettings": {"Networks": {}},
                        "Mounts": []
                    })
                })
                .collect();
            Json(result).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"message": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct LogsQuery {
    pub follow: Option<String>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub tail: Option<String>,
}

pub async fn logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<LogsQuery>,
) -> Response {
    let follow = q.follow.as_deref() == Some("1") || q.follow.as_deref() == Some("true");

    let (tx, rx) = mpsc::channel(64);
    let runtime = state.runtime.clone();
    let id_clone = id.clone();

    tokio::spawn(async move {
        let _ = runtime.stream_logs(&id_clone, follow, tx).await;
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

#[derive(Deserialize)]
pub struct StopQuery {
    pub t: Option<u32>,
}

pub async fn stop(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<StopQuery>,
) -> Response {
    let timeout = q.t.unwrap_or(10);
    match state.runtime.stop_container(&id, timeout).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
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

pub async fn wait(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.runtime.wait_container(&id).await {
        Ok(exit_code) => Json(json!({"StatusCode": exit_code})).into_response(),
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

#[derive(Deserialize)]
pub struct RemoveQuery {
    pub force: Option<String>,
    pub v: Option<String>,
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(_q): Query<RemoveQuery>,
) -> Response {
    match state.runtime.remove_container(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
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
