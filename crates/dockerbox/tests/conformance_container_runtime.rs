//! Conformance tests for the `ContainerRuntime` trait contract.
//!
//! Verifies:
//! - `ping()` returns `Ok(())` when implementation is healthy.
//! - `pull_image()` accepts a sender channel and completes.
//! - `image_exists()` returns `Ok(bool)` for any image reference.
//! - `create_container()` returns a unique ID on success.
//! - `start_container()` returns `NotFound` for unknown containers.
//! - `inspect_container()` returns `ContainerDetails` matching created config.
//! - `list_containers()` returns empty vec on fresh runtime.
//! - `list_containers()` returns created containers.
//! - `stream_logs()` returns error for non-existent container.
//! - `stop_container()` updates container status.
//! - `wait_container()` returns exit code.
//! - `remove_container()` removes container from list.
//! - `exec_create()` requires existing container.
//! - `exec_start()` requires existing exec_id.
//! - `exec_inspect()` returns `ExecDetails` for executed command.
//!
//! No daemon process, no network.

use async_trait::async_trait;
use dockerbox::domain::{
    ContainerDetails, ContainerRuntime, ContainerSummary, CreateConfig, ExecConfig, ExecDetails,
    LogChunk, PullProgress, RuntimeError, CgroupNs,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Mock runtime implementation
// ---------------------------------------------------------------------------

struct MockContainerRuntime {
    containers: Mutex<HashMap<String, ContainerDetails>>,
    exec_cache: Mutex<HashMap<String, (String, bool)>>, // exec_id -> (container_id, running)
}

impl MockContainerRuntime {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            containers: Mutex::new(HashMap::new()),
            exec_cache: Mutex::new(HashMap::new()),
        })
    }

    fn create_mock_details(id: &str, config: &CreateConfig) -> ContainerDetails {
        ContainerDetails {
            id: id.to_string(),
            name: config.name.clone().unwrap_or_else(|| format!("/{}", id)),
            image: config.image.clone(),
            status: "created".to_string(),
            exit_code: None,
            created: chrono::Utc::now().to_rfc3339(),
            config: config.clone(),
        }
    }
}

#[async_trait]
impl ContainerRuntime for MockContainerRuntime {
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
        let details = Self::create_mock_details(&id, &config);
        self.containers
            .lock()
            .expect("containers mutex poisoned")
            .insert(id.clone(), details);
        Ok(id)
    }

    async fn start_container(&self, id: &str) -> Result<(), RuntimeError> {
        let mut containers = self.containers.lock().expect("containers mutex poisoned");
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
            .expect("containers mutex poisoned")
            .get(id)
            .cloned()
            .ok_or_else(|| RuntimeError::NotFound(id.to_string()))
    }

    async fn list_containers(&self, _all: bool) -> Result<Vec<ContainerSummary>, RuntimeError> {
        let containers = self.containers.lock().expect("containers mutex poisoned");
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
            .expect("containers mutex poisoned")
            .contains_key(id)
        {
            Ok(())
        } else {
            Err(RuntimeError::NotFound(id.to_string()))
        }
    }

    async fn stop_container(&self, id: &str, _timeout_secs: u32) -> Result<(), RuntimeError> {
        let mut containers = self.containers.lock().expect("containers mutex poisoned");
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
            .expect("containers mutex poisoned")
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
            .expect("containers mutex poisoned")
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
            .expect("containers mutex poisoned")
            .contains_key(container_id)
        {
            let exec_id = uuid::Uuid::new_v4().to_string();
            self.exec_cache
                .lock()
                .expect("exec_cache mutex poisoned")
                .insert(exec_id.clone(), (container_id.to_string(), false));
            Ok(exec_id)
        } else {
            Err(RuntimeError::NotFound(container_id.to_string()))
        }
    }

    async fn exec_start(
        &self,
        exec_id: &str,
        _tx: mpsc::Sender<LogChunk>,
    ) -> Result<(), RuntimeError> {
        let mut cache = self.exec_cache.lock().expect("exec_cache mutex poisoned");
        if let Some((_container_id, running)) = cache.get_mut(exec_id) {
            *running = true;
            Ok(())
        } else {
            Err(RuntimeError::NotFound(exec_id.to_string()))
        }
    }

    async fn exec_inspect(&self, exec_id: &str) -> Result<ExecDetails, RuntimeError> {
        let cache = self.exec_cache.lock().expect("exec_cache mutex poisoned");
        if let Some((_container_id, running)) = cache.get(exec_id) {
            Ok(ExecDetails {
                id: exec_id.to_string(),
                exit_code: if *running { None } else { Some(0) },
                running: *running,
            })
        } else {
            Err(RuntimeError::NotFound(exec_id.to_string()))
        }
    }
}

// ---------------------------------------------------------------------------
// Conformance tests — trait contract
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ping_returns_ok() {
    let runtime = MockContainerRuntime::new();
    let result = runtime.ping().await;
    assert!(result.is_ok(), "ping must return Ok(())");
}

#[tokio::test]
async fn create_container_returns_non_empty_id() {
    let runtime = MockContainerRuntime::new();
    let config = CreateConfig {
        image: "alpine".to_string(),
        name: None,
        cmd: vec!["/bin/sh".to_string()],
        env: vec![],
        binds: vec![],
        ports: vec![],
        privileged: false,
        cgroup_ns: CgroupNs::Private,
        network_mode: None,
        labels: Default::default(),
    };
    let id = runtime.create_container(config).await.expect("create failed");
    assert!(!id.is_empty(), "container ID must not be empty");
}

#[tokio::test]
async fn create_container_returns_unique_ids() {
    let runtime = MockContainerRuntime::new();
    let config = CreateConfig {
        image: "alpine".to_string(),
        name: None,
        cmd: vec!["/bin/sh".to_string()],
        env: vec![],
        binds: vec![],
        ports: vec![],
        privileged: false,
        cgroup_ns: CgroupNs::Private,
        network_mode: None,
        labels: Default::default(),
    };
    let id1 = runtime
        .create_container(config.clone())
        .await
        .expect("create 1 failed");
    let id2 = runtime
        .create_container(config)
        .await
        .expect("create 2 failed");
    assert_ne!(id1, id2, "container IDs must be unique");
}

#[tokio::test]
async fn start_container_unknown_returns_not_found() {
    let runtime = MockContainerRuntime::new();
    let result = runtime.start_container("no-such-id").await;
    assert!(
        matches!(result, Err(RuntimeError::NotFound(_))),
        "start of unknown container must return NotFound"
    );
}

#[tokio::test]
async fn inspect_container_unknown_returns_not_found() {
    let runtime = MockContainerRuntime::new();
    let result = runtime.inspect_container("no-such-id").await;
    assert!(
        matches!(result, Err(RuntimeError::NotFound(_))),
        "inspect of unknown container must return NotFound"
    );
}

#[tokio::test]
async fn inspect_container_returns_details_matching_config() {
    let runtime = MockContainerRuntime::new();
    let config = CreateConfig {
        image: "alpine:latest".to_string(),
        name: Some("test-container".to_string()),
        cmd: vec!["/bin/echo".to_string(), "hello".to_string()],
        env: vec!["PATH=/bin".to_string()],
        binds: vec![],
        ports: vec![],
        privileged: true,
        cgroup_ns: CgroupNs::Host,
        network_mode: Some("host".to_string()),
        labels: Default::default(),
    };
    let id = runtime
        .create_container(config.clone())
        .await
        .expect("create failed");

    let details = runtime
        .inspect_container(&id)
        .await
        .expect("inspect failed");

    assert_eq!(details.id, id, "ID must match");
    assert_eq!(details.image, "alpine:latest", "image must match config");
    assert_eq!(details.name, "test-container", "name must match config");
    assert_eq!(details.status, "created", "initial status must be created");
}

#[tokio::test]
async fn list_containers_empty_on_fresh_runtime() {
    let runtime = MockContainerRuntime::new();
    let containers = runtime
        .list_containers(false)
        .await
        .expect("list failed");
    assert!(containers.is_empty(), "fresh runtime must have no containers");
}

#[tokio::test]
async fn list_containers_returns_created() {
    let runtime = MockContainerRuntime::new();
    let config = CreateConfig {
        image: "alpine".to_string(),
        name: Some("container1".to_string()),
        cmd: vec!["/bin/sh".to_string()],
        env: vec![],
        binds: vec![],
        ports: vec![],
        privileged: false,
        cgroup_ns: CgroupNs::Private,
        network_mode: None,
        labels: Default::default(),
    };
    let id = runtime
        .create_container(config)
        .await
        .expect("create failed");

    let containers = runtime
        .list_containers(false)
        .await
        .expect("list failed");
    assert_eq!(containers.len(), 1, "list must contain created container");
    assert_eq!(containers[0].id, id, "listed container ID must match");
}

#[tokio::test]
async fn stop_container_updates_status() {
    let runtime = MockContainerRuntime::new();
    let config = CreateConfig {
        image: "alpine".to_string(),
        name: None,
        cmd: vec!["/bin/sh".to_string()],
        env: vec![],
        binds: vec![],
        ports: vec![],
        privileged: false,
        cgroup_ns: CgroupNs::Private,
        network_mode: None,
        labels: Default::default(),
    };
    let id = runtime
        .create_container(config)
        .await
        .expect("create failed");
    runtime.start_container(&id).await.expect("start failed");

    runtime
        .stop_container(&id, 10)
        .await
        .expect("stop failed");

    let details = runtime
        .inspect_container(&id)
        .await
        .expect("inspect failed");
    assert_eq!(details.status, "exited", "stopped container must have exited status");
}

#[tokio::test]
async fn wait_container_unknown_returns_not_found() {
    let runtime = MockContainerRuntime::new();
    let result = runtime.wait_container("no-such-id").await;
    assert!(
        matches!(result, Err(RuntimeError::NotFound(_))),
        "wait of unknown container must return NotFound"
    );
}

#[tokio::test]
async fn wait_container_returns_exit_code() {
    let runtime = MockContainerRuntime::new();
    let config = CreateConfig {
        image: "alpine".to_string(),
        name: None,
        cmd: vec!["/bin/sh".to_string()],
        env: vec![],
        binds: vec![],
        ports: vec![],
        privileged: false,
        cgroup_ns: CgroupNs::Private,
        network_mode: None,
        labels: Default::default(),
    };
    let id = runtime
        .create_container(config)
        .await
        .expect("create failed");

    let exit_code = runtime.wait_container(&id).await.expect("wait failed");
    assert_eq!(exit_code, 0, "wait must return exit code");
}

#[tokio::test]
async fn remove_container_unknown_returns_not_found() {
    let runtime = MockContainerRuntime::new();
    let result = runtime.remove_container("no-such-id").await;
    assert!(
        matches!(result, Err(RuntimeError::NotFound(_))),
        "remove of unknown container must return NotFound"
    );
}

#[tokio::test]
async fn remove_container_deletes_from_list() {
    let runtime = MockContainerRuntime::new();
    let config = CreateConfig {
        image: "alpine".to_string(),
        name: None,
        cmd: vec!["/bin/sh".to_string()],
        env: vec![],
        binds: vec![],
        ports: vec![],
        privileged: false,
        cgroup_ns: CgroupNs::Private,
        network_mode: None,
        labels: Default::default(),
    };
    let id = runtime
        .create_container(config)
        .await
        .expect("create failed");
    runtime.remove_container(&id).await.expect("remove failed");

    let result = runtime.inspect_container(&id).await;
    assert!(
        matches!(result, Err(RuntimeError::NotFound(_))),
        "removed container must not be inspectable"
    );
}

#[tokio::test]
async fn exec_create_requires_existing_container() {
    let runtime = MockContainerRuntime::new();
    let config = ExecConfig {
        cmd: vec!["/bin/sh".to_string()],
        env: vec![],
        attach_stdout: true,
        attach_stderr: true,
    };
    let result = runtime.exec_create("no-such-container", config).await;
    assert!(
        matches!(result, Err(RuntimeError::NotFound(_))),
        "exec_create on unknown container must return NotFound"
    );
}

#[tokio::test]
async fn exec_create_returns_unique_ids() {
    let runtime = MockContainerRuntime::new();
    let create_config = CreateConfig {
        image: "alpine".to_string(),
        name: None,
        cmd: vec!["/bin/sh".to_string()],
        env: vec![],
        binds: vec![],
        ports: vec![],
        privileged: false,
        cgroup_ns: CgroupNs::Private,
        network_mode: None,
        labels: Default::default(),
    };
    let container_id = runtime
        .create_container(create_config)
        .await
        .expect("create failed");

    let exec_config = ExecConfig {
        cmd: vec!["/bin/sh".to_string()],
        env: vec![],
        attach_stdout: true,
        attach_stderr: true,
    };
    let exec_id1 = runtime
        .exec_create(&container_id, exec_config.clone())
        .await
        .expect("exec_create 1 failed");
    let exec_id2 = runtime
        .exec_create(&container_id, exec_config)
        .await
        .expect("exec_create 2 failed");

    assert_ne!(exec_id1, exec_id2, "exec IDs must be unique");
}

#[tokio::test]
async fn exec_start_requires_valid_exec_id() {
    let runtime = MockContainerRuntime::new();
    let (_tx, _rx) = mpsc::channel(1);
    let result = runtime.exec_start("no-such-exec", _tx).await;
    assert!(
        matches!(result, Err(RuntimeError::NotFound(_))),
        "exec_start with invalid ID must return NotFound"
    );
}

#[tokio::test]
async fn exec_inspect_returns_details() {
    let runtime = MockContainerRuntime::new();
    let create_config = CreateConfig {
        image: "alpine".to_string(),
        name: None,
        cmd: vec!["/bin/sh".to_string()],
        env: vec![],
        binds: vec![],
        ports: vec![],
        privileged: false,
        cgroup_ns: CgroupNs::Private,
        network_mode: None,
        labels: Default::default(),
    };
    let container_id = runtime
        .create_container(create_config)
        .await
        .expect("create failed");

    let exec_config = ExecConfig {
        cmd: vec!["/bin/sh".to_string()],
        env: vec![],
        attach_stdout: true,
        attach_stderr: true,
    };
    let exec_id = runtime
        .exec_create(&container_id, exec_config)
        .await
        .expect("exec_create failed");

    let details = runtime
        .exec_inspect(&exec_id)
        .await
        .expect("exec_inspect failed");

    assert_eq!(details.id, exec_id, "exec ID must match");
    assert!(!details.running, "fresh exec must not be running");
}

#[tokio::test]
async fn stream_logs_requires_valid_container() {
    let runtime = MockContainerRuntime::new();
    let (_tx, _rx) = mpsc::channel(1);
    let result = runtime.stream_logs("no-such-id", false, _tx).await;
    assert!(
        matches!(result, Err(RuntimeError::NotFound(_))),
        "stream_logs on unknown container must return NotFound"
    );
}

#[tokio::test]
async fn image_exists_returns_bool() {
    let runtime = MockContainerRuntime::new();
    let result = runtime.image_exists("alpine").await;
    assert!(result.is_ok(), "image_exists must return Ok(bool)");
    assert!(result.unwrap(), "image_exists must return true for any image");
}

#[tokio::test]
async fn pull_image_completes() {
    let runtime = MockContainerRuntime::new();
    let (_tx, _rx) = mpsc::channel(1);
    let result = runtime.pull_image("alpine", "latest", _tx).await;
    assert!(result.is_ok(), "pull_image must complete successfully");
}
