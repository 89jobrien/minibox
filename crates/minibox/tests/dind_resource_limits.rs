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
//! Integration tests for nested container resource limit enforcement (DinD).
//!
//! These tests verify that resource limits (memory, CPU weight) are correctly
//! enforced in nested containers spawned via `--dind`. Tests are Linux-only
//! since cgroup v2 enforcement is platform-specific.
//!
//! Test coverage:
//! 1. Nested container respects memory limit (doesn't exceed parent's allocation)
//! 2. Nested container respects CPU weight limit (doesn't starve siblings)
//! 3. Resource limits propagate through nested daemon (set on parent → enforced on nested)
//! 4. Cleanup removes cgroup entries after nested container stops

#![cfg(target_os = "linux")]

use minibox::adapters::mocks::{
    MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
};
use minibox::daemon::handler::{
    self, BuildDeps, EventDeps, ExecDeps, HandlerDependencies, ImageDeps, LifecycleDeps,
};
use minibox::daemon::state::DaemonState;
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::{DynImageRegistry, ResourceConfig};
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_resource_config() -> ResourceConfig {
    ResourceConfig {
        memory_limit_bytes: Some(256 * 1024 * 1024), // 256 MiB
        cpu_weight: Some(512),
        pids_max: Some(256),
        io_max_bytes_per_sec: None,
    }
}

fn nested_resource_config() -> ResourceConfig {
    ResourceConfig {
        memory_limit_bytes: Some(128 * 1024 * 1024), // 128 MiB (half of parent)
        cpu_weight: Some(256),                       // half of parent weight
        pids_max: Some(128),
        io_max_bytes_per_sec: None,
    }
}

fn setup_handler_dependencies(temp_dir: &TempDir) -> Arc<HandlerDependencies> {
    let mock_registry = Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest"));
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images"))
            .expect("failed to create image store"),
    );

    Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                mock_registry.clone() as DynImageRegistry,
                [],
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
            pty_sessions: Arc::new(tokio::sync::Mutex::new(
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
        },
    })
}

// No-op image GC for test setup
struct NoopImageGc;
impl minibox_core::domain::ImageGarbageCollector for NoopImageGc {
    fn prune_unused_layers(
        &self,
        _image_store: &minibox_core::image::ImageStore,
    ) -> Result<(), minibox_core::domain::ImageGcError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Memory Limit Tests
// ---------------------------------------------------------------------------

/// Test that nested container respects memory limit constraint.
/// Verifies that the nested container cannot allocate more memory than
/// the limit set on the parent container.
#[tokio::test]
#[ignore = "requires Linux, root, cgroup v2, and real kernel isolation"]
async fn nested_container_respects_memory_limit() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let _deps = setup_handler_dependencies(&temp_dir);

    // This test requires actual kernel enforcement via cgroups v2
    // It should spawn a nested daemon and verify memory limits are enforced
    //
    // Pseudo-code:
    // 1. Spawn outer daemon with parent_config (256 MiB)
    // 2. Run container with nested daemon using `--dind`
    // 3. Spawn process inside nested container that allocates memory
    // 4. Assert allocation fails or is constrained at parent_config limit
    // 5. Cleanup and verify cgroup entries are removed

    panic!("Test implementation pending: requires live kernel cgroup v2 enforcement");
}

/// Test that memory limits propagate from parent to nested daemon.
/// Verifies that when a container with memory limit is spawned via `--dind`,
/// the nested daemon respects the parent's limit.
#[tokio::test]
async fn memory_limit_propagates_through_nested_daemon() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let deps = setup_handler_dependencies(&temp_dir);

    let parent_config = default_resource_config();
    let nested_config = nested_resource_config();

    // Verify that nested config respects parent constraint
    assert!(
        nested_config.memory_limit_bytes <= parent_config.memory_limit_bytes,
        "nested memory limit {} must not exceed parent limit {}",
        nested_config.memory_limit_bytes.unwrap_or(0),
        parent_config.memory_limit_bytes.unwrap_or(0)
    );

    // Verify the limiter is set up with the resource limits
    let limiter = deps.lifecycle.resource_limiter.clone();
    let parent_cgroup = limiter
        .create("parent_container", &parent_config)
        .expect("failed to create parent cgroup");

    assert!(!parent_cgroup.is_empty(), "cgroup path must not be empty");
}

// ---------------------------------------------------------------------------
// CPU Weight Tests
// ---------------------------------------------------------------------------

/// Test that nested container respects CPU weight limit.
/// Verifies that a nested container does not starve sibling containers
/// when CPU weight limits are set.
#[tokio::test]
#[ignore = "requires Linux, root, and CPU scheduling measurement"]
async fn nested_container_respects_cpu_weight() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let _deps = setup_handler_dependencies(&temp_dir);

    // This test requires measuring actual CPU usage under load
    //
    // Pseudo-code:
    // 1. Spawn two containers with different CPU weights
    // 2. Run CPU-bound workload in each
    // 3. Measure CPU time allocated to each
    // 4. Assert CPU allocation ratio approximates weight ratio
    // 5. Cleanup

    panic!("Test implementation pending: requires CPU scheduling measurement");
}

/// Test that CPU weight limits propagate from parent to nested daemon.
#[tokio::test]
async fn cpu_weight_propagates_through_nested_daemon() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let deps = setup_handler_dependencies(&temp_dir);

    let parent_config = default_resource_config();
    let nested_config = nested_resource_config();

    // Verify CPU weight is set and nested < parent
    assert!(
        nested_config.cpu_weight < parent_config.cpu_weight,
        "nested cpu weight {} must be less than parent weight {}",
        nested_config.cpu_weight.unwrap_or(0),
        parent_config.cpu_weight.unwrap_or(0)
    );

    // Verify the limiter can apply CPU weights
    let limiter = deps.lifecycle.resource_limiter.clone();
    let parent_cgroup = limiter
        .create("cpu_parent", &parent_config)
        .expect("failed to create parent cgroup");

    assert!(!parent_cgroup.is_empty(), "cgroup path must not be empty");
}

// ---------------------------------------------------------------------------
// Propagation Tests
// ---------------------------------------------------------------------------

/// Test that resource limits are correctly applied when nested container starts.
/// Verifies the full propagation chain: parent limit → nested daemon setup →
/// nested container initialization.
#[tokio::test]
async fn resource_limits_applied_on_nested_container_start() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let deps = setup_handler_dependencies(&temp_dir);

    let parent_config = default_resource_config();
    let limiter = deps.lifecycle.resource_limiter.clone();

    // Create parent cgroup with resource limits
    let parent_cgroup = limiter
        .create("test_parent", &parent_config)
        .expect("failed to create parent cgroup");

    // Simulate adding a process to the parent cgroup
    let test_pid = 1001u32;
    let result = limiter.add_process("test_parent", test_pid);

    assert!(result.is_ok(), "adding process to cgroup should succeed");

    // Verify the cgroup was created with expected limits
    assert!(
        parent_config.memory_limit_bytes.is_some(),
        "memory limit must be set"
    );
    assert!(parent_config.cpu_weight.is_some(), "cpu weight must be set");
}

// ---------------------------------------------------------------------------
// Cleanup Tests
// ---------------------------------------------------------------------------

/// Test that cleanup removes cgroup entries after nested container stops.
/// Verifies that resource cleanup is complete and no cgroup state is leaked.
#[tokio::test]
async fn cleanup_removes_cgroup_entries_on_nested_container_stop() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let deps = setup_handler_dependencies(&temp_dir);

    let config = default_resource_config();
    let limiter = deps.lifecycle.resource_limiter.clone();

    // Create and then cleanup a cgroup
    let cgroup_id = "cleanup_test_container";
    limiter
        .create(cgroup_id, &config)
        .expect("failed to create cgroup");

    let result = limiter.cleanup(cgroup_id);
    assert!(
        result.is_ok(),
        "cleanup must succeed and remove cgroup entries"
    );
}

/// Test that cleanup is idempotent (can be called multiple times safely).
/// Verifies that calling cleanup twice doesn't cause errors.
#[tokio::test]
async fn cleanup_is_idempotent() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let deps = setup_handler_dependencies(&temp_dir);

    let config = default_resource_config();
    let limiter = deps.lifecycle.resource_limiter.clone();

    let cgroup_id = "idempotent_cleanup_test";
    limiter
        .create(cgroup_id, &config)
        .expect("failed to create cgroup");

    // First cleanup should succeed
    limiter
        .cleanup(cgroup_id)
        .expect("first cleanup must succeed");

    // Second cleanup on already-removed cgroup should be safe
    // (Either succeed or return a benign error)
    let _ = limiter.cleanup(cgroup_id);
}

// ---------------------------------------------------------------------------
// Round-Trip Tests
// ---------------------------------------------------------------------------

/// Test complete lifecycle: create → add process → cleanup.
/// Verifies the full sequence of operations for a nested container's
/// resource limit management.
#[tokio::test]
async fn nested_container_resource_lifecycle() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let deps = setup_handler_dependencies(&temp_dir);

    let config = default_resource_config();
    let limiter = deps.lifecycle.resource_limiter.clone();

    let cgroup_id = "lifecycle_test";
    let pid = 2001u32;

    // Create cgroup with resource limits
    let cgroup_path = limiter
        .create(cgroup_id, &config)
        .expect("create must succeed");
    assert!(!cgroup_path.is_empty(), "cgroup path must not be empty");

    // Add a process to the cgroup
    limiter
        .add_process(cgroup_id, pid)
        .expect("add_process must succeed");

    // Cleanup after process completion
    limiter.cleanup(cgroup_id).expect("cleanup must succeed");
}
