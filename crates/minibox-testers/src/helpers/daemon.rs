//! Shared helpers for daemonbox conformance tests.

use crate::helpers::gc::NoopImageGc;
use crate::mocks::MockRegistry;
use crate::mocks::{MockFilesystem, MockLimiter, MockNetwork, MockRuntime};
use daemonbox::handler::{
    BuildDeps, EventDeps, ExecDeps, HandlerDependencies, ImageDeps, LifecycleDeps, NoopImageLoader,
    PtySessionRegistry,
};
use daemonbox::state::DaemonState;
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
    })
}

/// Build mock [`DaemonState`] rooted under `base`.
pub fn make_mock_state(base: &Path) -> Arc<DaemonState> {
    let image_store = linuxbox::image::ImageStore::new(base.join("images")).unwrap();
    Arc::new(DaemonState::new(image_store, base))
}
