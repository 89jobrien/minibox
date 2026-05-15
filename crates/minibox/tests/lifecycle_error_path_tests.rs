//! Error-path coverage for container lifecycle flows (GH #116).
//!
//! Targets the branches not already exercised by
//! `daemon_container_lifecycle_failure_tests.rs` and
//! `daemon_handler_failure_tests.rs`:
//!
//! - `handle_pause` when the cgroup.freeze write fails (no such path on disk)
//! - `handle_resume` on a Running container (state-machine violation)
//! - `handle_resume` when the cgroup.freeze write fails
//! - `handle_remove` on a Running container (must be rejected)
//! - `handle_remove` when filesystem cleanup fails (best-effort, remove succeeds)
//! - `handle_remove` when cgroup cleanup fails (best-effort, remove succeeds)

use minibox::adapters::mocks::{
    MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
};
use minibox::daemon::handler;
use minibox::daemon::state::{ContainerRecord, DaemonState};
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
    let image_store =
        minibox::image::ImageStore::new(temp_dir.path().join("images")).expect("image store");
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
) -> Arc<minibox::daemon::handler::HandlerDependencies> {
    use minibox::daemon::handler::{BuildDeps, EventDeps, ExecDeps, ImageDeps, LifecycleDeps};

    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images2")).expect("image store"),
    );
    Arc::new(minibox::daemon::handler::HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                registry as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
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
                minibox::daemon::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(minibox::daemon::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: minibox::daemon::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
        checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
    })
}

/// Build a ContainerRecord with the given state string.
///
/// `cgroup_path` should be set to a path that does NOT exist so that
/// cgroup write operations fail deterministically.
fn make_record_with_state(id: &str, state: &str, cgroup_path: PathBuf) -> ContainerRecord {
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: state.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            pid: None,
        },
        pid: None,
        rootfs_path: PathBuf::from("/tmp/fake-rootfs"),
        cgroup_path,
        post_exit_hooks: vec![],
        rootfs_metadata: None,
        source_image_ref: None,
        step_state: None,
        priority: None,
        urgency: None,
        execution_context: None,
        creation_params: None,
        manifest_path: None,
        workload_digest: None,
    }
}

fn noop_event_sink() -> Arc<dyn minibox_core::events::EventSink> {
    Arc::new(minibox_core::events::NoopEventSink)
}

// ---------------------------------------------------------------------------
// handle_pause error paths
// ---------------------------------------------------------------------------

/// handle_pause on a Running container whose cgroup.freeze path does not exist
/// returns a DaemonResponse::Error containing "pause failed".
///
/// This covers the `tokio::fs::write` failure branch in `handle_pause`.
#[tokio::test]
async fn test_handle_pause_cgroup_write_fails_returns_error() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let state = make_state(&temp_dir);

    // Point cgroup_path to a directory that does not exist so the write
    // to cgroup.freeze fails with ENOENT. Insert directly as "Running" so
    // the state-machine guard in handle_pause passes without a transition.
    let bad_cgroup = temp_dir.path().join("nonexistent-cgroup").join("minibox");
    let id = "pause-cgroup-fail-001";
    let record = make_record_with_state(id, "Running", bad_cgroup);
    state.add_container(record).await;

    let resp = handler::handle_pause(id.to_string(), state, noop_event_sink()).await;

    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("pause failed")),
        "expected 'pause failed' error when cgroup write fails, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_resume error paths
// ---------------------------------------------------------------------------

/// handle_resume on a Running container (not paused) returns an error
/// containing "not paused".
///
/// This covers the state-machine guard in `handle_resume` for the wrong-state
/// branch (the "is not paused" path).
#[tokio::test]
async fn test_handle_resume_running_container_returns_not_paused_error() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let state = make_state(&temp_dir);

    let cgroup_path = temp_dir.path().join("cgroup-running");
    let id = "resume-running-001";
    // Insert directly as "Running" — no transition needed.
    // handle_resume checks state == "Paused", so this exercises the wrong-state branch.
    let record = make_record_with_state(id, "Running", cgroup_path);
    state.add_container(record).await;

    let resp = handler::handle_resume(id.to_string(), state, noop_event_sink()).await;

    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("not paused")),
        "expected 'not paused' error when resuming a Running container, got {resp:?}"
    );
}

/// handle_resume on a Paused container whose cgroup.freeze path does not exist
/// returns a DaemonResponse::Error containing "resume failed".
///
/// This covers the `tokio::fs::write` failure branch in `handle_resume`.
#[tokio::test]
async fn test_handle_resume_cgroup_write_fails_returns_error() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let state = make_state(&temp_dir);

    // Bad cgroup path — the directory does not exist.
    let bad_cgroup = temp_dir.path().join("nonexistent-cgroup2").join("minibox");
    let id = "resume-cgroup-fail-001";
    // Insert directly as "Paused" so handle_resume's state check passes.
    let record = make_record_with_state(id, "Paused", bad_cgroup);
    state.add_container(record).await;

    let resp = handler::handle_resume(id.to_string(), state, noop_event_sink()).await;

    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("resume failed")),
        "expected 'resume failed' error when cgroup write fails, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_remove error paths
// ---------------------------------------------------------------------------

/// handle_remove on a Running container is rejected with an error.
///
/// The `remove_inner` function checks `record.info.state == "Running"` and
/// returns `Err(DomainError::AlreadyRunning)`. This ensures we cover that
/// guard branch.
#[tokio::test]
async fn test_handle_remove_running_container_returns_error() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let state = make_state(&temp_dir);

    let id = "remove-running-001";
    let cgroup_path = temp_dir.path().join("cgroup-remove-running");
    // Insert directly as "Running" — remove_inner checks state == "Running" and rejects.
    let record = make_record_with_state(id, "Running", cgroup_path);
    state.add_container(record).await;

    let runtime = Arc::new(MockRuntime::new());
    let filesystem = Arc::new(MockFilesystem::new());
    let registry = Arc::new(MockRegistry::new());
    let deps = make_deps(&temp_dir, runtime, filesystem, registry);

    let resp = handler::handle_remove(id.to_string(), state.clone(), deps).await;

    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "expected Error when removing a Running container, got {resp:?}"
    );
    // Container must NOT have been removed from state.
    let still_present = state.get_container(id).await;
    assert!(
        still_present.is_some(),
        "Running container must remain in state after rejected remove"
    );
}

/// handle_remove succeeds even when the filesystem cleanup adapter fails.
///
/// `remove_inner` treats filesystem cleanup failure as best-effort — it logs a
/// warning but continues. The container must be deregistered from state and the
/// response must be `Success`.
#[tokio::test]
async fn test_handle_remove_filesystem_cleanup_failure_is_best_effort() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let state = make_state(&temp_dir);

    let id = "remove-fs-fail-001";
    // Create the container dir so the `container_dir.exists()` check fires and
    // the cleanup call is actually attempted.
    let containers_base = temp_dir.path().join("containers");
    std::fs::create_dir_all(containers_base.join(id)).expect("create container dir");

    let cgroup_path = temp_dir.path().join("cgroup-fs-fail");
    let mut record = make_record_with_state(id, "Stopped", cgroup_path);
    record.info.state = "Stopped".to_string();
    state.add_container(record).await;

    // Use a MockFilesystem configured to fail cleanup.
    let filesystem = Arc::new(MockFilesystem::new().with_cleanup_failure());
    let runtime = Arc::new(MockRuntime::new());
    let registry = Arc::new(MockRegistry::new());

    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images_fsfail"))
            .expect("image store"),
    );
    let deps = Arc::new(minibox::daemon::handler::HandlerDependencies {
        image: minibox::daemon::handler::ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                registry as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: minibox::daemon::handler::LifecycleDeps {
            filesystem,
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime,
            network_provider: Arc::new(MockNetwork::new()),
            containers_base,
            run_containers_base: temp_dir.path().join("run"),
        },
        exec: minibox::daemon::handler::ExecDeps {
            exec_runtime: None,
            pty_sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
                minibox::daemon::handler::PtySessionRegistry::default(),
            )),
        },
        build: minibox::daemon::handler::BuildDeps {
            image_pusher: None,
            commit_adapter: None,
            image_builder: None,
        },
        events: minibox::daemon::handler::EventDeps {
            event_sink: Arc::new(minibox_core::events::NoopEventSink),
            event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
            metrics: Arc::new(minibox::daemon::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: minibox::daemon::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
        checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
    });

    let resp = handler::handle_remove(id.to_string(), state.clone(), deps).await;

    // Despite cleanup failure, remove should succeed (best-effort).
    assert!(
        matches!(resp, DaemonResponse::Success { .. }),
        "expected Success even when filesystem cleanup fails (best-effort), got {resp:?}"
    );
    // Container must be deregistered from state.
    let gone = state.get_container(id).await;
    assert!(
        gone.is_none(),
        "container must be removed from state even when filesystem cleanup fails"
    );
}

/// handle_remove succeeds even when the cgroup cleanup adapter fails.
///
/// `remove_inner` treats cgroup cleanup failure as best-effort — it logs a
/// warning but continues. The container must be deregistered from state.
#[tokio::test]
async fn test_handle_remove_cgroup_cleanup_failure_is_best_effort() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let state = make_state(&temp_dir);

    let id = "remove-cgroup-fail-001";
    let cgroup_path = temp_dir.path().join("cgroup-cleanup-fail");
    let mut record = make_record_with_state(id, "Stopped", cgroup_path);
    record.info.state = "Stopped".to_string();
    state.add_container(record).await;

    // Use a MockLimiter configured to fail cleanup.
    let limiter = Arc::new(MockLimiter::new().with_cleanup_failure());
    let runtime = Arc::new(MockRuntime::new());
    let filesystem = Arc::new(MockFilesystem::new());
    let registry = Arc::new(MockRegistry::new());

    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images_cgfail"))
            .expect("image store"),
    );
    let deps = Arc::new(minibox::daemon::handler::HandlerDependencies {
        image: minibox::daemon::handler::ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                registry as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: minibox::daemon::handler::LifecycleDeps {
            filesystem,
            resource_limiter: limiter,
            runtime,
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers"),
            run_containers_base: temp_dir.path().join("run"),
        },
        exec: minibox::daemon::handler::ExecDeps {
            exec_runtime: None,
            pty_sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
                minibox::daemon::handler::PtySessionRegistry::default(),
            )),
        },
        build: minibox::daemon::handler::BuildDeps {
            image_pusher: None,
            commit_adapter: None,
            image_builder: None,
        },
        events: minibox::daemon::handler::EventDeps {
            event_sink: Arc::new(minibox_core::events::NoopEventSink),
            event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
            metrics: Arc::new(minibox::daemon::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: minibox::daemon::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
        checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
    });

    let resp = handler::handle_remove(id.to_string(), state.clone(), deps).await;

    // Despite cgroup cleanup failure, remove should succeed (best-effort).
    assert!(
        matches!(resp, DaemonResponse::Success { .. }),
        "expected Success even when cgroup cleanup fails (best-effort), got {resp:?}"
    );
    // Container must be deregistered from state.
    let gone = state.get_container(id).await;
    assert!(
        gone.is_none(),
        "container must be removed from state even when cgroup cleanup fails"
    );
}
