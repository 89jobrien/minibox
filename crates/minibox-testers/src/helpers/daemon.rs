//! Shared helpers for daemon conformance tests.

use crate::helpers::gc::NoopImageGc;
use crate::mocks::MockRegistry;
use crate::mocks::{MockFilesystem, MockLimiter, MockNetwork, MockRuntime};
use minibox::daemon::handler::{
    BuildDeps, ContainerPolicy, EventDeps, ExecDeps, HandlerDependencies, ImageDeps, LifecycleDeps,
    NoopImageLoader, PtySessionRegistry,
};
use minibox::daemon::state::DaemonState;
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::DynImageRegistry;
use minibox_core::events::{BroadcastEventBroker, NoopEventSink};
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

/// Build a [`HandlerDependencies`] wired with mock adapters, rooted under
/// `temp_dir`.
pub fn make_mock_deps(temp_dir: &TempDir) -> Arc<HandlerDependencies> {
    make_mock_deps_with_registry(MockRegistry::new(), temp_dir)
}

/// Build mock deps with a specific [`ContainerPolicy`].
///
/// Use this when testing policy gate behavior (bind mount / privileged denials).
/// The default policy from [`make_mock_deps`] permits both; this variant lets
/// you pass a deny-all or partially-restricted policy.
///
/// ```rust,ignore
/// let deps = make_mock_deps_with_policy(
///     &tmp,
///     ContainerPolicy { allow_bind_mounts: false, allow_privileged: false },
/// );
/// ```
pub fn make_mock_deps_with_policy(
    temp_dir: &TempDir,
    policy: ContainerPolicy,
) -> Arc<HandlerDependencies> {
    let base = make_mock_deps_with_registry(MockRegistry::new(), temp_dir);
    // SAFETY: Arc::try_unwrap would fail here since we just created it — clone
    // the inner value and rebuild with the new policy.
    let mut deps = (*base).clone();
    deps.policy = policy;
    Arc::new(deps)
}

/// Build mock deps with a specific `registry`.
pub fn make_mock_deps_with_registry(
    registry: MockRegistry,
    temp_dir: &TempDir,
) -> Arc<HandlerDependencies> {
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("img")).unwrap());
    Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(registry) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(NoopImageLoader),
            image_gc: Arc::new(NoopImageGc::new()),
            image_store,
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers"),
            run_containers_base: temp_dir.path().join("run"),
        },
        exec: ExecDeps {
            exec_runtime: None,
            pty_sessions: Arc::new(tokio::sync::Mutex::new(PtySessionRegistry::default())),
        },
        build: BuildDeps {
            image_pusher: None,
            commit_adapter: None,
            image_builder: None,
        },
        events: EventDeps {
            event_sink: Arc::new(NoopEventSink),
            event_source: Arc::new(BroadcastEventBroker::new()),
            metrics: Arc::new(minibox::daemon::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: minibox::daemon::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
    })
}

/// Build mock [`DaemonState`] rooted under `base`.
pub fn make_mock_state(base: &Path) -> Arc<DaemonState> {
    let image_store = minibox::image::ImageStore::new(base.join("images")).unwrap();
    Arc::new(DaemonState::new(image_store, base))
}

/// Build a minimal [`minibox::daemon::state::ContainerRecord`] for use in tests.
///
/// All optional/path fields are set to safe empty/tmp values. The returned
/// record is in `Created` state with no PID.
pub fn make_stub_record(id: impl Into<String>) -> minibox::daemon::state::ContainerRecord {
    let id = id.into();
    minibox::daemon::state::ContainerRecord {
        info: minibox_core::protocol::ContainerInfo {
            id: id.clone(),
            name: None,
            image: "test:latest".to_string(),
            command: "/bin/true".to_string(),
            state: "created".to_string(),
            created_at: "1970-01-01T00:00:00Z".to_string(),
            pid: None,
        },
        pid: None,
        // Use ID-namespaced paths under /tmp to avoid collisions in parallel test runs.
        rootfs_path: std::path::PathBuf::from(format!("/tmp/minibox-test-{id}-rootfs")),
        cgroup_path: std::path::PathBuf::from(format!("/tmp/minibox-test-{id}-cgroup")),
        post_exit_hooks: vec![],
        rootfs_metadata: None,
        source_image_ref: Some("test:latest".to_string()),
    }
}

/// Build a [`DaemonState`] pre-populated with `n` stub container records.
///
/// Uses `tokio::runtime::Runtime::new()` internally — do not call from within
/// an existing async context (use `make_mock_state` + `add_container` instead).
pub fn make_mock_state_with_n_containers(base: &Path, n: usize) -> Arc<DaemonState> {
    let state = make_mock_state(base);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        for i in 0..n {
            state
                .add_container(make_stub_record(format!("ctr-{i:04}")))
                .await;
        }
    });
    state
}
