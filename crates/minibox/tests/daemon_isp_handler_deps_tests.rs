#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::redundant_field_names,
    clippy::uninlined_format_args,
    clippy::redundant_clone,
    clippy::redundant_closure,
    clippy::single_char_pattern,
    clippy::unwrap_in_result,
    clippy::collapsible_if,
    clippy::match_same_arms,
    clippy::only_used_in_recursion,
    clippy::used_underscore_binding,
    clippy::map_unwrap_or,
    clippy::manual_assert,
    clippy::as_ptr_cast_mut,
    clippy::ptr_as_ptr,
    clippy::must_use_candidate,
    clippy::used_underscore_items,
    clippy::missing_const_for_fn,
    clippy::manual_string_new,
    clippy::semicolon_if_nothing_returned,
    clippy::unreadable_literal,
    clippy::default_constructed_unit_structs,
    clippy::ref_as_ptr,
    clippy::allow_attributes_without_reason,
    clippy::redundant_closure_for_method_calls,
    clippy::needless_raw_string_hashes,
    clippy::manual_is_variant_and,
    clippy::ignore_without_reason,
    clippy::default_trait_access,
    clippy::cast_lossless,
    clippy::match_wild_err_arm,
    clippy::format_push_string,
    clippy::bool_assert_comparison,
    clippy::struct_excessive_bools
)]
//! Tests for ISP-compliant HandlerDependencies sub-struct decomposition.
//!
//! Verifies that HandlerDependencies exposes focused sub-structs so each
//! handler can depend only on the slice of infrastructure it actually needs.

use minibox::adapters::mocks::{
    MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
};
use minibox::daemon::handler::{
    BuildDeps, EventDeps, ExecDeps, HandlerDependencies, ImageDeps, LifecycleDeps,
};
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
    let tmp = TempDir::new().expect("unwrap in test");
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(tmp.path().join("images")).expect("unwrap in test"),
    );

    let image_deps = ImageDeps {
        registry_router: Arc::new(HostnameRegistryRouter::new(
            Arc::new(MockRegistry::new()) as DynImageRegistry,
            std::iter::empty::<(&str, DynImageRegistry)>(),
        )),
        image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
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
            minibox::daemon::handler::PtySessionRegistry::default(),
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
        metrics: Arc::new(minibox::daemon::telemetry::NoOpMetricsRecorder::new()),
    };

    let deps = HandlerDependencies {
        image: image_deps,
        lifecycle: lifecycle_deps,
        exec: exec_deps,
        build: build_deps,
        events: event_deps,
        policy: minibox::daemon::handler::ContainerPolicy::default(),
        execution_policy: None,
        checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
    };

    // Verify sub-struct fields are accessible
    let _ = &deps.image.image_gc; // field is accessible
    let _ = deps.lifecycle.containers_base;
    assert!(deps.exec.exec_runtime.is_none());
    assert!(deps.build.image_pusher.is_none());
}
