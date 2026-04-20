//! Tests for ISP-compliant HandlerDependencies sub-struct decomposition.
//!
//! Verifies that HandlerDependencies exposes focused sub-structs so each
//! handler can depend only on the slice of infrastructure it actually needs.

use daemonbox::handler::{
    BuildDeps, EventDeps, ExecDeps, HandlerDependencies, ImageDeps, LifecycleDeps,
};
use mbx::adapters::mocks::{MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime};
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::DynImageRegistry;
use std::sync::Arc;
use tempfile::TempDir;

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

/// Build a `HandlerDependencies` using the new sub-struct fields.
///
/// This test fails before the decomposition is implemented because `ImageDeps`,
/// `LifecycleDeps`, `ExecDeps`, `BuildDeps`, and `EventDeps` do not exist yet.
#[test]
fn handler_deps_are_accessible_via_sub_structs() {
    let tmp = TempDir::new().unwrap();
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(tmp.path().join("images")).unwrap());

    let image_deps = ImageDeps {
        registry_router: Arc::new(HostnameRegistryRouter::new(
            Arc::new(MockRegistry::new()) as DynImageRegistry,
            std::iter::empty::<(&str, DynImageRegistry)>(),
        )),
        image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
        image_gc: Arc::new(NoopImageGc),
        image_store,
    };

    let lifecycle_deps = LifecycleDeps {
        filesystem: Arc::new(MockFilesystem::new()),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new()),
        network_provider: Arc::new(MockNetwork::new()),
        containers_base: tmp.path().join("containers"),
        run_containers_base: tmp.path().join("run"),
    };

    let exec_deps = ExecDeps {
        exec_runtime: None,
        pty_sessions: Arc::new(tokio::sync::Mutex::new(
            daemonbox::handler::PtySessionRegistry::default(),
        )),
    };

    let build_deps = BuildDeps {
        image_pusher: None,
        commit_adapter: None,
        image_builder: None,
    };

    let event_deps = EventDeps {
        event_sink: Arc::new(minibox_core::events::NoopEventSink),
        event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
        metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
    };

    let deps = HandlerDependencies {
        image: image_deps,
        lifecycle: lifecycle_deps,
        exec: exec_deps,
        build: build_deps,
        events: event_deps,
        policy: daemonbox::handler::ContainerPolicy::default(),
    };

    // Verify sub-struct fields are accessible
    let _ = &deps.image.image_gc; // field is accessible
    let _ = deps.lifecycle.containers_base;
    assert!(deps.exec.exec_runtime.is_none());
    assert!(deps.build.image_pusher.is_none());
}
