use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use super::AppState;
use crate::infra::state::NetworkRecord;

#[derive(Deserialize)]
pub struct CreateNetworkBody {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Driver", default = "default_bridge")]
    pub driver: String,
}

fn default_bridge() -> String {
    "bridge".to_string()
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateNetworkBody>,
) -> impl IntoResponse {
    let id = Uuid::new_v4().to_string().replace('-', "");
    let record = NetworkRecord {
        id: id.clone(),
        name: body.name,
        driver: body.driver,
        created: Utc::now().to_rfc3339(),
    };
    state
        .state
        .networks
        .write()
        .await
        .insert(id.clone(), record);
    (StatusCode::CREATED, Json(json!({"Id": id, "Warning": ""})))
}

pub async fn list(State(state): State<AppState>) -> impl IntoResponse {
    let networks = state.state.networks.read().await;
    let result: Vec<_> = networks
        .values()
        .map(|n| {
            json!({
                "Id": n.id,
                "Name": n.name,
                "Driver": n.driver,
                "Created": n.created,
                "Scope": "local",
                "IPAM": {"Driver": "default", "Config": []},
                "Internal": false,
                "Attachable": false,
                "Ingress": false,
                "Containers": {},
                "Options": {},
                "Labels": {}
            })
        })
        .collect();
    Json(result)
}

pub async fn inspect(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let networks = state.state.networks.read().await;
    // Look up by id or name
    let record = networks
        .values()
        .find(|n| n.id == id || n.name == id)
        .cloned();

    match record {
        Some(n) => Json(json!({
            "Id": n.id,
            "Name": n.name,
            "Driver": n.driver,
            "Created": n.created,
            "Scope": "local",
            "IPAM": {"Driver": "default", "Config": []},
            "Internal": false,
            "Attachable": false,
            "Ingress": false,
            "Containers": {},
            "Options": {},
            "Labels": {}
        }))
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"message": format!("network {} not found", id)})),
        )
            .into_response(),
    }
}

pub async fn remove(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let mut networks = state.state.networks.write().await;
    let key = networks
        .values()
        .find(|n| n.id == id || n.name == id)
        .map(|n| n.id.clone());

    match key {
        Some(k) => {
            networks.remove(&k);
            StatusCode::NO_CONTENT.into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"message": format!("network {} not found", id)})),
        )
            .into_response(),
    }
}
