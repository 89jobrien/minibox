use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::AppState;

#[derive(Deserialize)]
pub struct PullQuery {
    #[serde(rename = "fromImage")]
    pub from_image: Option<String>,
    pub tag: Option<String>,
}

pub async fn pull(State(state): State<AppState>, Query(q): Query<PullQuery>) -> Response {
    let image = q.from_image.unwrap_or_else(|| "library/ubuntu".to_string());
    let tag = q.tag.unwrap_or_else(|| "latest".to_string());

    let (tx, rx) = mpsc::channel(32);
    let runtime = state.runtime.clone();
    let image_clone = image.clone();
    let tag_clone = tag.clone();

    tokio::spawn(async move {
        let _ = runtime.pull_image(&image_clone, &tag_clone, tx).await;
    });

    let stream = ReceiverStream::new(rx).map(move |progress| {
        let obj = json!({
            "status": progress.status,
            "id": progress.id,
            "progressDetail": {},
            "progress": progress.progress
        });
        let mut line = serde_json::to_vec(&obj).unwrap_or_default();
        line.push(b'\n');
        Ok::<_, std::convert::Infallible>(bytes::Bytes::from(line))
    });

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from_stream(stream))
        .unwrap()
}

#[derive(Deserialize)]
pub struct ImageNamePath {
    pub name: String,
}

pub async fn inspect(State(state): State<AppState>, Path(name): Path<String>) -> Response {
    match state.runtime.image_exists(&name).await {
        Ok(true) => {
            let body = json!({
                "Id": format!("sha256:{:0<64}", name.replace(':', "")),
                "RepoTags": [name],
                "RepoDigests": [],
                "Created": "2024-01-01T00:00:00Z",
                "Size": 0,
                "VirtualSize": 0,
                "GraphDriver": {"Name": "overlay2", "Data": null},
                "RootFS": {"Type": "layers", "Layers": []},
                "Metadata": {"LastTagTime": "0001-01-01T00:00:00Z"}
            });
            Json(body).into_response()
        }
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({"message": format!("No such image: {}", name)})),
        )
            .into_response(),
    }
}
