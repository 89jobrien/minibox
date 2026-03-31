use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde_json::json;

use super::AppState;

pub async fn ping() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

pub async fn version() -> impl IntoResponse {
    Json(json!({
        "Version": "20.10.0",
        "ApiVersion": "1.41",
        "MinAPIVersion": "1.12",
        "GitCommit": "dockerbox",
        "GoVersion": "go1.21.0",
        "Os": "linux",
        "Arch": "amd64",
        "KernelVersion": "5.15.0",
        "BuildTime": "2024-01-01T00:00:00.000000000+00:00"
    }))
}

pub async fn info(State(state): State<AppState>) -> impl IntoResponse {
    let ping_ok = state.runtime.ping().await.is_ok();
    Json(json!({
        "ID": "dockerbox",
        "Containers": 0,
        "ContainersRunning": 0,
        "ContainersPaused": 0,
        "ContainersStopped": 0,
        "Images": 0,
        "Driver": "overlay2",
        "MemoryLimit": true,
        "SwapLimit": false,
        "KernelMemory": false,
        "CpuCfsPeriod": true,
        "CpuCfsQuota": true,
        "CPUShares": true,
        "CPUSet": false,
        "IPv4Forwarding": true,
        "BridgeNfIptables": false,
        "BridgeNfIp6tables": false,
        "Debug": false,
        "OomKillDisable": false,
        "NGoroutines": 0,
        "LoggingDriver": "json-file",
        "CgroupDriver": "cgroupfs",
        "DockerRootDir": "/var/lib/dockerbox",
        "HttpProxy": "",
        "HttpsProxy": "",
        "NoProxy": "",
        "Name": "dockerbox",
        "ServerVersion": "20.10.0",
        "OperatingSystem": "linux",
        "OSType": "linux",
        "Architecture": "x86_64",
        "NCPU": 1,
        "MemTotal": 0,
        "IndexServerAddress": "https://index.docker.io/v1/",
        "RegistryConfig": {},
        "GenericResources": null,
        "HttpProxy_": "",
        "minibox_available": ping_ok
    }))
}
