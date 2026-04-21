//! Shared helpers for daemonbox conformance tests.
//!
//! This module defines [`TestBackendDescriptor`], which describes a concrete
//! handler-level backend under conformance test with boolean capability flags.
//!
//! # Capability flags
//!
//! Unlike the [`minibox_core::adapters::conformance::BackendDescriptor`] (which
//! covers commit/build/push adapter traits), [`TestBackendDescriptor`] covers the
//! **handler-level** operations exercised through [`daemonbox::handler`]:
//!
//! | Flag              | Meaning                                      |
//! |-------------------|----------------------------------------------|
//! | `supports_run`    | Backend can spawn containers via `handle_run` |
//! | `supports_commit` | Backend has a `commit_adapter` wired          |
//! | `supports_build`  | Backend has an `image_builder` wired          |
//! | `supports_push`   | Backend has an `image_pusher` wired           |
//!
//! Tests check the relevant flag and return early (skip) when `false`, consistent
//! with the conformance suite skip-not-fail convention.
//!
//! # Constructor hooks
//!
//! Each `make_*` field holds an optional `Box<dyn Fn()>` factory that produces a
//! fresh adapter instance. This mirrors the design of `BackendDescriptor` in
//! `minibox-core`: construction is deferred until the test actually needs the
//! adapter, and each invocation returns a fresh instance.

use daemonbox::handler::{
    BuildDeps, EventDeps, ExecDeps, HandlerDependencies, ImageDeps, LifecycleDeps, NoopImageLoader,
    PtySessionRegistry,
};
use daemonbox::state::DaemonState;
use minibox_testers::mocks::{
    MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
};
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::DynImageRegistry;
use minibox_core::events::{BroadcastEventBroker, NoopEventSink};
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// TestBackendDescriptor
// ---------------------------------------------------------------------------

/// Describes a daemonbox handler-level backend under conformance test.
///
/// Boolean capability flags control which conformance tests are exercised.
/// Tests must check the relevant flag and return early when it is `false`
/// (skip, not fail).
pub struct TestBackendDescriptor {
    /// Human-readable name used in test failure messages.
    pub name: &'static str,

    /// `true` when the backend supports `handle_run` (spawn containers).
    pub supports_run: bool,

    /// `true` when the backend has a `commit_adapter` wired in
    /// [`HandlerDependencies::build`].
    pub supports_commit: bool,

    /// `true` when the backend has an `image_builder` wired in
    /// [`HandlerDependencies::build`].
    pub supports_build: bool,

    /// `true` when the backend has an `image_pusher` wired in
    /// [`HandlerDependencies::build`].
    pub supports_push: bool,

    /// Factory for a fresh [`HandlerDependencies`] backed by mock adapters.
    ///
    /// The closure receives a `&TempDir` so all path-based fields can be
    /// rooted in a single temporary directory owned by the caller.
    pub make_deps: Box<dyn Fn(&TempDir) -> Arc<HandlerDependencies> + Send + Sync>,
}

impl TestBackendDescriptor {
    /// Create a descriptor for the default mock-backed backend.
    ///
    /// `supports_run` is `true`; commit/build/push are `false` (no adapters
    /// wired).  Use [`TestBackendDescriptor::with_run`] and friends to
    /// override individual flags.
    pub fn mock_backend(name: &'static str) -> Self {
        Self {
            name,
            supports_run: true,
            supports_commit: false,
            supports_build: false,
            supports_push: false,
            make_deps: Box::new(|temp_dir| make_mock_deps(temp_dir)),
        }
    }

    /// Override `supports_run`.
    pub fn with_run(mut self, val: bool) -> Self {
        self.supports_run = val;
        self
    }

    /// Override `supports_commit`.
    pub fn with_commit(mut self, val: bool) -> Self {
        self.supports_commit = val;
        self
    }

    /// Override `supports_build`.
    pub fn with_build(mut self, val: bool) -> Self {
        self.supports_build = val;
        self
    }

    /// Override `supports_push`.
    pub fn with_push(mut self, val: bool) -> Self {
        self.supports_push = val;
        self
    }

    /// Build a `HandlerDependencies` using the descriptor's factory.
    pub fn build_deps(&self, temp_dir: &TempDir) -> Arc<HandlerDependencies> {
        (self.make_deps)(temp_dir)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

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
            image_gc: Arc::new(NoopImageGc),
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
    let image_store = minibox::image::ImageStore::new(base.join("images")).unwrap();
    Arc::new(DaemonState::new(image_store, base))
}

use minibox_testers::helpers::NoopImageGc;
