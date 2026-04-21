//! Dockerbox API conformance tests.
//!
//! Exercises the axum router with a mock `ContainerRuntime` — no daemon
//! required.  Each test verifies that the HTTP shim speaks the Docker v1.41
//! wire format correctly: status codes, required JSON fields, and round-trip
//! semantics for the create → start → list → stop → remove lifecycle.

use async_trait::async_trait;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;
use tower::ServiceExt as _;

// Re-use the crate's internal types by depending on the crate under test.
use dockerbox::{
    api::{AppState, router},
    domain::{
        ContainerDetails, ContainerRuntime, ContainerSummary, CreateConfig, ExecConfig,
        ExecDetails, LogChunk, PullProgress, RuntimeError,
    },
    infra::state::StateStore,
};

// ---------------------------------------------------------------------------
// Mock runtime
// ---------------------------------------------------------------------------

#[derive(Default)]
struct MockRuntime {
    containers: Mutex<HashMap<String, ContainerDetails>>,
}

impl MockRuntime {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

fn mock_details(id: &str, config: &CreateConfig) -> ContainerDetails {
    ContainerDetails {
        id: id.to_string(),
        name: config.name.clone().unwrap_or_else(|| format!("/{id}")),
        image: config.image.clone(),
        status: "created".to_string(),
        exit_code: None,
        created: chrono::Utc::now().to_rfc3339(),
        config: config.clone(),
    }
}

#[async_trait]
impl ContainerRuntime for MockRuntime {
    async fn ping(&self) -> Result<(), RuntimeError> {
        Ok(())
    }

    async fn pull_image(
        &self,
        _image: &str,
        _tag: &str,
        _tx: mpsc::Sender<PullProgress>,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }

    async fn image_exists(&self, _image: &str) -> Result<bool, RuntimeError> {
        Ok(true)
    }

    async fn create_container(&self, config: CreateConfig) -> Result<String, RuntimeError> {
        let id = uuid::Uuid::new_v4().to_string();
        let details = mock_details(&id, &config);
        self.containers
            .lock()
            .expect("MockRuntime mutex poisoned")
            .insert(id.clone(), details);
        Ok(id)
    }

    async fn start_container(&self, id: &str) -> Result<(), RuntimeError> {
        let mut containers = self.containers.lock().expect("MockRuntime mutex poisoned");
        if let Some(c) = containers.get_mut(id) {
            c.status = "running".to_string();
            Ok(())
        } else {
            Err(RuntimeError::NotFound(id.to_string()))
        }
    }

    async fn inspect_container(&self, id: &str) -> Result<ContainerDetails, RuntimeError> {
        self.containers
            .lock()
            .expect("MockRuntime mutex poisoned")
            .get(id)
            .cloned()
            .ok_or_else(|| RuntimeError::NotFound(id.to_string()))
    }

    async fn list_containers(&self, _all: bool) -> Result<Vec<ContainerSummary>, RuntimeError> {
        let containers = self.containers.lock().expect("MockRuntime mutex poisoned");
        Ok(containers
            .values()
            .map(|c| ContainerSummary {
                id: c.id.clone(),
                names: vec![c.name.clone()],
                image: c.image.clone(),
                status: c.status.clone(),
                state: c.status.clone(),
            })
            .collect())
    }

    async fn stream_logs(
        &self,
        id: &str,
        _follow: bool,
        _tx: mpsc::Sender<LogChunk>,
    ) -> Result<(), RuntimeError> {
        if self
            .containers
            .lock()
            .expect("MockRuntime mutex poisoned")
            .contains_key(id)
        {
            Ok(())
        } else {
            Err(RuntimeError::NotFound(id.to_string()))
        }
    }

    async fn stop_container(&self, id: &str, _timeout_secs: u32) -> Result<(), RuntimeError> {
        let mut containers = self.containers.lock().expect("MockRuntime mutex poisoned");
        if let Some(c) = containers.get_mut(id) {
            c.status = "exited".to_string();
            Ok(())
        } else {
            Err(RuntimeError::NotFound(id.to_string()))
        }
    }

    async fn wait_container(&self, id: &str) -> Result<i64, RuntimeError> {
        if self
            .containers
            .lock()
            .expect("MockRuntime mutex poisoned")
            .contains_key(id)
        {
            Ok(0)
        } else {
            Err(RuntimeError::NotFound(id.to_string()))
        }
    }

    async fn remove_container(&self, id: &str) -> Result<(), RuntimeError> {
        self.containers
            .lock()
            .expect("MockRuntime mutex poisoned")
            .remove(id)
            .map(|_| ())
            .ok_or_else(|| RuntimeError::NotFound(id.to_string()))
    }

    async fn exec_create(
        &self,
        container_id: &str,
        _config: ExecConfig,
    ) -> Result<String, RuntimeError> {
        if self
            .containers
            .lock()
            .expect("MockRuntime mutex poisoned")
            .contains_key(container_id)
        {
            Ok(uuid::Uuid::new_v4().to_string())
        } else {
            Err(RuntimeError::NotFound(container_id.to_string()))
        }
    }

    async fn exec_start(
        &self,
        _exec_id: &str,
        _tx: mpsc::Sender<LogChunk>,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }

    async fn exec_inspect(&self, exec_id: &str) -> Result<ExecDetails, RuntimeError> {
        Err(RuntimeError::NotFound(exec_id.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn make_app() -> (axum::Router, Arc<MockRuntime>) {
    let runtime = MockRuntime::new();
    let state = AppState {
        runtime: runtime.clone(),
        state: StateStore::default(),
    };
    (router(state), runtime)
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).expect("response body is not valid JSON")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// `GET /_ping` returns 200 OK with body "OK".
#[tokio::test]
async fn ping_returns_200() {
    let (app, _) = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/_ping")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

/// `GET /version` returns 200 with required Docker version fields.
#[tokio::test]
async fn version_returns_required_fields() {
    let (app, _) = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/version")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["Version"].is_string(), "Version field missing");
    assert!(json["ApiVersion"].is_string(), "ApiVersion field missing");
}

/// `POST /containers/create` returns 201 with an `Id` field.
#[tokio::test]
async fn create_container_returns_id() {
    let (app, _) = make_app();
    let body = serde_json::to_vec(&serde_json::json!({
        "Image": "alpine",
        "Cmd": ["/bin/sh"],
    }))
    .unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/containers/create")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    assert!(
        json["Id"].is_string(),
        "Id field missing in create response"
    );
}

/// Full lifecycle: create → start → list → stop → remove.
#[tokio::test]
async fn container_lifecycle_roundtrip() {
    let (app, _runtime) = make_app();

    // Create
    let body = serde_json::to_vec(&serde_json::json!({
        "Image": "alpine",
        "Cmd": ["/bin/echo", "hello"],
    }))
    .unwrap();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/containers/create")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let create_json = body_json(resp).await;
    let id = create_json["Id"].as_str().unwrap().to_string();

    // Start
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/containers/{id}/start"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NO_CONTENT,
        "start should return 204"
    );

    // List — container should appear
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/containers/json")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let list_json = body_json(resp).await;
    let ids: Vec<&str> = list_json
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["Id"].as_str())
        .collect();
    assert!(ids.contains(&id.as_str()), "created container not in list");

    // Stop
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/containers/{id}/stop"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NO_CONTENT,
        "stop should return 204"
    );

    // Verify state is exited via HTTP inspect
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/containers/{id}/json"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let inspect_json = body_json(resp).await;
    assert_eq!(
        inspect_json["State"]["Status"].as_str().unwrap_or(""),
        "exited",
        "container status should be exited after stop"
    );

    // Remove
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/containers/{id}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NO_CONTENT,
        "remove should return 204"
    );

    // Container should be gone — 404 via HTTP
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/containers/{id}/json"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "container should be gone after remove"
    );
}

/// `GET /containers/json` on an empty runtime returns an empty array.
#[tokio::test]
async fn list_empty_returns_array() {
    let (app, _) = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/containers/json")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json.is_array(), "empty list should return JSON array");
}
