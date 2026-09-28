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
//! Shared test helpers for daemon handler integration tests.
//!
//! Import this module in each split test file via `mod daemon_handler_common`.
#![allow(dead_code)]

use minibox::adapters::mocks::{
    MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
};
use minibox::daemon::handler::{
    BuildDeps, EventDeps, ExecDeps, HandlerDependencies, ImageDeps, LifecycleDeps,
};
use minibox::daemon::state::DaemonState;
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::DynImageRegistry;
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// NoopImageGc
// ---------------------------------------------------------------------------

/// No-op image GC for tests.
pub struct NoopImageGc;

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

// ---------------------------------------------------------------------------
// handle_run_once
// ---------------------------------------------------------------------------

/// Helper that calls `handle_run` via a channel and returns the first response.
///
/// `handle_run` sends responses via a channel rather than returning them,
/// so tests use this wrapper to recover the single response for non-ephemeral runs.
#[allow(clippy::too_many_arguments)]
pub async fn handle_run_once(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    ephemeral: bool,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    use minibox::daemon::handler;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        handler::RunParams {
            image: image,
            tag: tag,
            command: command,
            memory_limit_bytes: memory_limit_bytes,
            cpu_weight: cpu_weight,
            ephemeral: ephemeral,
            network: None,
            mounts: vec![],
            privileged: false,
            shared_uid_range: false,
            env: vec![],
            name: None,
            platform: None,
            cgroup_parent: None,
            priority: None,
            policy_override: None,
        },
        state,
        deps,
        tx,
    )
    .await;
    rx.recv().await.expect("handler sent no response")
}

// ---------------------------------------------------------------------------
// extract_container_id
// ---------------------------------------------------------------------------

/// Extract container ID from a ContainerCreated response; panics otherwise.
pub fn extract_container_id(response: &DaemonResponse) -> String {
    match response {
        DaemonResponse::ContainerCreated { id } => id.clone(),
        other => panic!("expected ContainerCreated, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Dep builders
// ---------------------------------------------------------------------------

/// Low-level builder: creates `HandlerDependencies` with the given registry_router,
/// image_store, and network_provider; all other fields use sensible mock defaults.
pub fn build_deps(
    registry_router: minibox_core::domain::DynRegistryRouter,
    image_store: Arc<minibox_core::image::ImageStore>,
    network_provider: minibox_core::domain::DynNetworkProvider,
    containers_base: std::path::PathBuf,
    run_containers_base: std::path::PathBuf,
) -> Arc<HandlerDependencies> {
    Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router,
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider,
            containers_base,
            run_containers_base,
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
            ..Default::default()
        },
        execution_policy: None,
        checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
    })
}

/// Convenience wrapper: standard mock deps with a given registry and image_store.
pub fn build_deps_with_registry(
    registry_router: minibox_core::domain::DynRegistryRouter,
    image_store: Arc<minibox_core::image::ImageStore>,
    temp_dir: &TempDir,
) -> Arc<HandlerDependencies> {
    build_deps(
        registry_router,
        image_store,
        Arc::new(MockNetwork::new()),
        temp_dir.path().join("containers"),
        temp_dir.path().join("run"),
    )
}

/// Helper to create test dependencies with mocks.
pub fn create_test_deps_with_dir(temp_dir: &TempDir) -> Arc<HandlerDependencies> {
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images2"))
            .expect("unwrap in test"),
    );
    Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
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
            ..Default::default()
        },
        execution_policy: None,
        checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
    })
}

/// Helper to create test dependencies with a custom runtime.
///
/// Identical to [`create_test_deps_with_dir`] except the `runtime` field is
/// supplied by the caller.  Use this to inject a [`MockRuntime`] configured
/// with [`MockRuntime::with_output_pipe`] when testing streaming/ephemeral paths.
pub fn create_test_deps_with_dir_and_runtime(
    temp_dir: &TempDir,
    runtime: minibox_core::domain::DynContainerRuntime,
) -> Arc<HandlerDependencies> {
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images-rt"))
            .expect("unwrap in test"),
    );
    Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime,
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers-rt"),
            run_containers_base: temp_dir.path().join("run-rt"),
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
            ..Default::default()
        },
        execution_policy: None,
        checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
    })
}

/// Helper to create daemon state with a test image store.
pub fn create_test_state_with_dir(temp_dir: &TempDir) -> Arc<DaemonState> {
    let image_store =
        minibox::image::ImageStore::new(temp_dir.path().join("images")).expect("unwrap in test");
    Arc::new(DaemonState::new(image_store, temp_dir.path()))
}

/// Build deps with a specific `MockNetwork` instance so call counts can be
/// inspected after the handler runs.
pub fn create_test_deps_with_network(
    temp_dir: &TempDir,
    network: Arc<MockNetwork>,
) -> Arc<HandlerDependencies> {
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images3"))
            .expect("unwrap in test"),
    );
    Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: network,
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
            ..Default::default()
        },
        execution_policy: None,
        checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
    })
}

// ---------------------------------------------------------------------------
// make_deps_with_policy
// ---------------------------------------------------------------------------

/// Build deps with a specific `ContainerPolicy` configured.
pub fn make_deps_with_policy(
    temp_dir: &TempDir,
    policy: minibox::daemon::handler::ContainerPolicy,
) -> Arc<HandlerDependencies> {
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images_pol"))
            .expect("unwrap in test"),
    );
    Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers_pol"),
            run_containers_base: temp_dir.path().join("run_pol"),
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
        policy,
        execution_policy: None,
        checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
    })
}

// ---------------------------------------------------------------------------
// wait_for_container_state
// ---------------------------------------------------------------------------

/// Poll DaemonState until the container exists (any state), or the deadline
/// passes. Returns `true` if found.
pub async fn wait_for_container(state: &Arc<DaemonState>, id: &str, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if state.get_container(id).await.is_some() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }
}

/// Poll DaemonState until the container reaches the expected state or the
/// deadline passes.  Returns the final state string.
pub async fn wait_for_container_state(
    state: &Arc<DaemonState>,
    id: &str,
    expected: &str,
    timeout_ms: u64,
) -> String {
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);
    loop {
        if let Some(record) = state.get_container(id).await
            && record.info.state == expected
        {
            return record.info.state;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }
    state
        .get_container(id)
        .await
        .map(|r| r.info.state)
        .unwrap_or_else(|| "GONE".to_string())
}
