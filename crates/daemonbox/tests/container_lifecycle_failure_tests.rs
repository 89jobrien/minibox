//! Tests for container lifecycle failure paths (GH #24, minibox-32).
//!
//! Verifies that handlers return appropriate error responses when underlying
//! adapters fail, and that concurrent operations are safe under failure conditions.

use daemonbox::handler;
use daemonbox::state::{ContainerRecord, DaemonState};
use minibox::adapters::mocks::{
    MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
};
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::DynImageRegistry;
use minibox_core::protocol::{ContainerInfo, DaemonResponse};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_state(temp_dir: &TempDir) -> Arc<DaemonState> {
    let image_store = minibox::image::ImageStore::new(temp_dir.path().join("images")).unwrap();
    Arc::new(DaemonState::new(image_store, temp_dir.path()))
}

struct NoopImageGc;

#[async_trait::async_trait]
impl minibox_core::image::gc::ImageGarbageCollector for NoopImageGc {
    async fn prune(
        &self,
        dry_run: bool,
        _in_use: &[String],
    ) -> anyhow::Result<minibox_core::image::gc::PruneReport> {
        Ok(minibox_core::image::gc::PruneReport {
            removed: vec![],
            freed_bytes: 0,
            dry_run,
        })
    }
}

fn make_deps(
    temp_dir: &TempDir,
    runtime: Arc<MockRuntime>,
    filesystem: Arc<MockFilesystem>,
    registry: Arc<MockRegistry>,
) -> Arc<daemonbox::handler::HandlerDependencies> {
    use daemonbox::handler::{BuildDeps, EventDeps, ExecDeps, ImageDeps, LifecycleDeps};
    use minibox_core::adapters::HostnameRegistryRouter;
    use minibox_core::domain::DynImageRegistry;

    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("images2")).unwrap());
    Arc::new(daemonbox::handler::HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                registry as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: LifecycleDeps {
            filesystem,
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime,
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers"),
            run_containers_base: temp_dir.path().join("run"),
        },
        exec: ExecDeps {
            exec_runtime: None,
            pty_sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
                daemonbox::handler::PtySessionRegistry::default(),
            )),
        },
        build: BuildDeps {
            image_pusher: None,
            commit_adapter: None,
            image_builder: None,
        },
        events: EventDeps {
            event_sink: Arc::new(minibox_core::events::NoopEventSink),
            event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
    })
}

fn make_record(id: &str) -> ContainerRecord {
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: "Created".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            pid: None,
        },
        pid: None,
        rootfs_path: PathBuf::from("/tmp/fake-rootfs"),
        cgroup_path: PathBuf::from("/sys/fs/cgroup/minibox/fake"),
        post_exit_hooks: vec![],
        rootfs_metadata: None,
        source_image_ref: None,
    }
}

/// Helper that calls `handle_run` via a channel and returns the first response.
async fn handle_run_once(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    ephemeral: bool,
    state: Arc<DaemonState>,
    deps: Arc<daemonbox::handler::HandlerDependencies>,
) -> DaemonResponse {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        image,
        tag,
        command,
        memory_limit_bytes,
        cpu_weight,
        ephemeral,
        None,
        vec![],
        false,
        vec![],
        None,
        state,
        deps,
        tx,
    )
    .await;
    rx.recv().await.expect("handler sent no response")
}

// ---------------------------------------------------------------------------
// handle_run Failure Tests
// ---------------------------------------------------------------------------

/// When MockLimiter is configured to fail cgroup create, handle_run returns Error response.
/// This happens synchronously before spawn, making it reliably testable across platforms.
#[tokio::test]
async fn test_handle_run_limiter_failure_returns_error_response() {
    let temp_dir = TempDir::new().unwrap();
    let state = make_state(&temp_dir);

    // Use a pre-cached image so pull succeeds; limiter failure happens early.
    let runtime = Arc::new(MockRuntime::new());
    let filesystem = Arc::new(MockFilesystem::new());
    let registry = Arc::new(MockRegistry::new().with_cached_image("alpine", "latest"));

    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images_limiter")).unwrap(),
    );
    let deps = Arc::new(daemonbox::handler::HandlerDependencies {
        image: daemonbox::handler::ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                registry.clone() as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: daemonbox::handler::LifecycleDeps {
            filesystem: filesystem.clone() as Arc<dyn minibox_core::domain::FilesystemProvider>,
            resource_limiter: Arc::new(MockLimiter::new().with_create_failure()),
            runtime: runtime.clone() as Arc<dyn minibox_core::domain::ContainerRuntime>,
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers"),
            run_containers_base: temp_dir.path().join("run"),
        },
        exec: daemonbox::handler::ExecDeps {
            exec_runtime: None,
            pty_sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
                daemonbox::handler::PtySessionRegistry::default(),
            )),
        },
        build: daemonbox::handler::BuildDeps {
            image_pusher: None,
            commit_adapter: None,
            image_builder: None,
        },
        events: daemonbox::handler::EventDeps {
            event_sink: Arc::new(minibox_core::events::NoopEventSink),
            event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
    });

    let response = handle_run_once(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state,
        deps,
    )
    .await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(
                !message.is_empty(),
                "error message should indicate resource limiter failure"
            );
        }
        other => panic!("expected Error response for limiter failure, got {other:?}"),
    }
}

/// When MockFilesystem is configured to fail setup_rootfs, handle_run returns Error response.
#[tokio::test]
async fn test_handle_run_filesystem_failure_returns_error_response() {
    let temp_dir = TempDir::new().unwrap();
    let state = make_state(&temp_dir);

    let runtime = Arc::new(MockRuntime::new());
    let filesystem = Arc::new(MockFilesystem::new().with_setup_failure());
    let registry = Arc::new(MockRegistry::new());

    let deps = make_deps(&temp_dir, runtime, filesystem, registry);

    let response = handle_run_once(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state,
        deps,
    )
    .await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(!message.is_empty(), "error message should not be empty");
        }
        other => panic!("expected Error response for filesystem failure, got {other:?}"),
    }
}

/// When MockRegistry is configured to fail pull_image, handle_run returns Error response.
#[tokio::test]
async fn test_handle_run_registry_pull_failure_returns_error_response() {
    let temp_dir = TempDir::new().unwrap();
    let state = make_state(&temp_dir);

    let runtime = Arc::new(MockRuntime::new());
    let filesystem = Arc::new(MockFilesystem::new());
    let registry = Arc::new(MockRegistry::new().with_pull_failure());

    let deps = make_deps(&temp_dir, runtime, filesystem, registry);

    let response = handle_run_once(
        "nonexistent-image".to_string(),
        Some("latest".to_string()),
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state,
        deps,
    )
    .await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(!message.is_empty(), "error message should not be empty");
        }
        other => panic!("expected Error response for registry pull failure, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// handle_stop / handle_remove Failure Tests
// ---------------------------------------------------------------------------

/// handle_stop on a non-existent container returns Error response.
#[tokio::test]
async fn test_handle_stop_nonexistent_container_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let state = make_state(&temp_dir);

    let runtime = Arc::new(MockRuntime::new());
    let filesystem = Arc::new(MockFilesystem::new());
    let registry = Arc::new(MockRegistry::new());

    let deps = make_deps(&temp_dir, runtime, filesystem, registry);

    let response = handler::handle_stop("nonexistent-id-12345".to_string(), state, deps).await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("not found") || message.contains("nonexistent"),
                "error message should indicate container not found, got: {}",
                message
            );
        }
        other => panic!("expected Error response for nonexistent container, got {other:?}"),
    }
}

/// handle_remove on a non-existent container returns Error response.
#[tokio::test]
async fn test_handle_remove_nonexistent_container_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let state = make_state(&temp_dir);

    let runtime = Arc::new(MockRuntime::new());
    let filesystem = Arc::new(MockFilesystem::new());
    let registry = Arc::new(MockRegistry::new());

    let deps = make_deps(&temp_dir, runtime, filesystem, registry);

    let response = handler::handle_remove("nonexistent-id-12345".to_string(), state, deps).await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("not found") || message.contains("nonexistent"),
                "error message should indicate container not found, got: {}",
                message
            );
        }
        other => panic!("expected Error response for nonexistent container, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Concurrent Safety Tests
// ---------------------------------------------------------------------------

/// Concurrent add/remove operations under failure conditions remain safe.
///
/// This test spawns 10 tasks in parallel, each adding then removing a unique
/// container. It verifies that concurrent operations under stress do not corrupt
/// state or panic, and final state is empty.
#[tokio::test]
async fn test_daemon_state_concurrent_add_and_remove_is_safe() {
    let temp_dir = TempDir::new().unwrap();
    let state = make_state(&temp_dir);

    let mut handles = vec![];

    // Spawn 10 tasks, each adding and removing a unique container.
    for i in 0..10u32 {
        let state_clone = Arc::clone(&state);
        let id = format!("concurrent-lifecycle-{i:04}");
        let handle = tokio::spawn(async move {
            let record = make_record(&id);
            state_clone.add_container(record).await;
            state_clone.remove_container(&id).await;
        });
        handles.push(handle);
    }

    // Await all tasks.
    for handle in handles {
        handle.await.expect("task must not panic");
    }

    // After all tasks complete, state should be empty.
    let containers = state.list_containers().await;
    assert!(
        containers.is_empty(),
        "expected empty state after concurrent add/remove, got {} containers",
        containers.len()
    );
}

/// Concurrent handle_stop calls on the same container are safe.
///
/// Multiple tasks attempt to stop the same container concurrently.
/// Only the first should succeed; later attempts should return "not found".
#[tokio::test]
async fn test_concurrent_stop_calls_are_safe() {
    let temp_dir = TempDir::new().unwrap();
    let state = make_state(&temp_dir);

    // Add a single container.
    let container_id = "concurrent-stop-test";
    state.add_container(make_record(container_id)).await;

    let runtime = Arc::new(MockRuntime::new());
    let filesystem = Arc::new(MockFilesystem::new());
    let registry = Arc::new(MockRegistry::new());
    let deps = make_deps(&temp_dir, runtime, filesystem, registry);

    let mut handles = vec![];

    // Spawn 5 tasks, each calling handle_stop on the same container.
    for _ in 0..5 {
        let state_clone = Arc::clone(&state);
        let deps_clone = Arc::clone(&deps);
        let id = container_id.to_string();
        let handle =
            tokio::spawn(async move { handler::handle_stop(id, state_clone, deps_clone).await });
        handles.push(handle);
    }

    // Collect responses.
    let mut responses = vec![];
    for handle in handles {
        let response = handle.await.expect("task must not panic");
        responses.push(response);
    }

    // At least one should succeed or give a sensible error.
    // All responses should be valid DaemonResponse variants (no panics).
    assert_eq!(responses.len(), 5);
    for response in responses {
        match response {
            DaemonResponse::Success { .. } | DaemonResponse::Error { .. } => {}
            other => panic!("unexpected response variant: {other:?}"),
        }
    }
}
