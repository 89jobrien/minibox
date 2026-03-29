use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use super::AppState;
use crate::infra::state::VolumeRecord;

#[derive(Deserialize)]
pub struct CreateVolumeBody {
    #[serde(rename = "Name")]
    pub name: Option<String>,
    #[serde(rename = "Driver", default = "default_local")]
    pub driver: String,
}

fn default_local() -> String {
    "local".to_string()
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateVolumeBody>,
) -> impl IntoResponse {
    let name = body
        .name
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string().replace('-', ""));

    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let mountpoint = format!("{}/.local/share/dockerbox/volumes/{}", home, name);

    // Create the volume directory
    if let Err(e) = tokio::fs::create_dir_all(&mountpoint).await {
        tracing::warn!("failed to create volume dir {}: {}", mountpoint, e);
    }

    let record = VolumeRecord {
        name: name.clone(),
        driver: body.driver,
        mountpoint: mountpoint.clone(),
        created: Utc::now().to_rfc3339(),
    };

    state
        .state
        .volumes
        .write()
        .await
        .insert(name.clone(), record.clone());

    (StatusCode::CREATED, Json(volume_json(&record)))
}

pub async fn inspect(State(state): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    let volumes = state.state.volumes.read().await;
    match volumes.get(&name) {
        Some(v) => Json(volume_json(v)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"message": format!("No such volume: {}", name)})),
        )
            .into_response(),
    }
}

pub async fn remove(State(state): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    let mut volumes = state.state.volumes.write().await;
    match volumes.remove(&name) {
        Some(_) => StatusCode::NO_CONTENT.into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"message": format!("No such volume: {}", name)})),
        )
            .into_response(),
    }
}

fn volume_json(v: &VolumeRecord) -> serde_json::Value {
    json!({
        "Name": v.name,
        "Driver": v.driver,
        "Mountpoint": v.mountpoint,
        "CreatedAt": v.created,
        "Labels": {},
        "Scope": "local",
        "Options": {}
    })
}
