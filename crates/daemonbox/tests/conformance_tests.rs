//! Cross-platform conformance tests for minibox adapters.
//!
//! Ensures behavior parity across different adapter implementations
//! (Linux native, WSL2, Docker Desktop) and highlights OS-specific differences.
//!
//! **Purpose:** Validate hexagonal architecture abstraction doesn't leak
//! platform-specific behavior into domain logic.
// Issues #62, #67, #71: commit/build/push conformance tests added below.

use daemonbox::handler::{self, HandlerDependencies};
use daemonbox::state::{ContainerState, DaemonState};
use minibox::adapters::mocks::{MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime};
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::{
    ContainerHooks, ContainerRuntime, ContainerSpawnConfig, DynImageRegistry, ImageRegistry,
    NetworkConfig, NetworkMode, NetworkProvider, ResourceConfig, ResourceLimiter, RootfsSetup,
};
use minibox_core::protocol::DaemonResponse;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

/// No-op image GC for tests.
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

/// Helper that wraps `handle_run` with a channel, returning the first response.
#[allow(clippy::too_many_arguments)]
async fn handle_run_once(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    ephemeral: bool,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
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

/// Create a `HandlerDependencies` with mocks, using `temp_dir` for path fields.
fn mock_deps(temp_dir: &TempDir) -> Arc<HandlerDependencies> {
    mock_deps_with_registry(MockRegistry::new(), temp_dir)
}

fn mock_deps_with_registry(registry: MockRegistry, temp_dir: &TempDir) -> Arc<HandlerDependencies> {
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap());
    Arc::new(HandlerDependencies {
        image: daemonbox::handler::ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(registry) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: daemonbox::handler::LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
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
    })
}

fn mock_deps_with_network(
    network: std::sync::Arc<MockNetwork>,
    temp_dir: &TempDir,
) -> Arc<HandlerDependencies> {
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap());
    Arc::new(HandlerDependencies {
        image: daemonbox::handler::ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest"))
                    as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: daemonbox::handler::LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: network,
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
    })
}

fn mock_state(temp_dir: &TempDir) -> Arc<DaemonState> {
    let image_store = minibox::image::ImageStore::new(temp_dir.path().join("images")).unwrap();
    Arc::new(DaemonState::new(image_store, temp_dir.path()))
}

/// Conformance test suite for domain trait implementations.
///
/// All adapters (Linux, WSL, Docker Desktop) must pass these tests
/// to ensure behavioral parity.
mod conformance {
    use super::*;

    // -------------------------------------------------------------------------
    // ImageRegistry Conformance
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn registry_must_report_cached_images() {
        let registry = MockRegistry::new().with_cached_image("alpine", "latest");

        assert!(
            registry.has_image("alpine", "latest").await,
            "Registry must return true for cached images"
        );
        assert!(
            !registry.has_image("ubuntu", "latest").await,
            "Registry must return false for non-cached images"
        );
    }

    #[tokio::test]
    async fn registry_must_handle_pull_failures_gracefully() {
        let registry = MockRegistry::new().with_pull_failure();

        let image_ref = minibox::ImageRef::parse("alpine").unwrap();
        let result = registry.pull_image(&image_ref).await;
        assert!(
            result.is_err(),
            "Registry must return error for failed pulls"
        );
    }

    #[tokio::test]
    async fn registry_must_return_layer_paths_for_cached_images() {
        let registry = MockRegistry::new().with_cached_image("alpine", "latest");

        let layers = registry.get_image_layers("alpine", "latest");
        assert!(
            layers.is_ok(),
            "Registry must return layer paths for cached images"
        );

        let layer_paths = layers.unwrap();
        assert!(
            !layer_paths.is_empty(),
            "Registry must return non-empty layer paths"
        );
    }

    // -------------------------------------------------------------------------
    // FilesystemProvider Conformance
    // -------------------------------------------------------------------------

    #[test]
    fn filesystem_must_return_merged_directory() {
        let fs = MockFilesystem::new();
        let layers = vec![PathBuf::from("/layer1"), PathBuf::from("/layer2")];
        let container_dir = PathBuf::from("/container");

        let result = fs.setup_rootfs(&layers, &container_dir);
        assert!(result.is_ok(), "Filesystem must successfully setup rootfs");

        let merged = result.unwrap();
        assert!(
            merged.merged_dir.to_string_lossy().contains("merged"),
            "Filesystem must return merged directory path"
        );
    }

    #[test]
    fn filesystem_must_cleanup_without_error() {
        let fs = MockFilesystem::new();
        let container_dir = PathBuf::from("/container");

        let result = fs.cleanup(&container_dir);
        assert!(result.is_ok(), "Filesystem cleanup must not error");
    }

    // -------------------------------------------------------------------------
    // ResourceLimiter Conformance
    // -------------------------------------------------------------------------

    #[test]
    fn limiter_must_create_cgroup_and_return_path() {
        let limiter = MockLimiter::new();
        let config = ResourceConfig {
            memory_limit_bytes: Some(512 * 1024 * 1024),
            cpu_weight: Some(500),
            pids_max: Some(1024),
            io_max_bytes_per_sec: None,
        };

        let result = limiter.create("container-123", &config);
        assert!(result.is_ok(), "Limiter must create cgroup successfully");

        let path = result.unwrap();
        assert!(
            !path.is_empty(),
            "Limiter must return non-empty cgroup path"
        );
    }

    #[test]
    fn limiter_must_add_process_to_cgroup() {
        let limiter = MockLimiter::new();
        let config = ResourceConfig::default();

        limiter.create("container-123", &config).unwrap();
        let result = limiter.add_process("container-123", 12345);

        assert!(
            result.is_ok(),
            "Limiter must add process to cgroup without error"
        );
    }

    #[test]
    fn limiter_must_cleanup_cgroup() {
        let limiter = MockLimiter::new();
        let config = ResourceConfig::default();

        limiter.create("container-123", &config).unwrap();
        let result = limiter.cleanup("container-123");

        assert!(result.is_ok(), "Limiter must cleanup cgroup without error");
    }

    // -------------------------------------------------------------------------
    // ContainerRuntime Conformance
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn runtime_must_return_valid_pid() {
        let runtime = MockRuntime::new();
        let config = ContainerSpawnConfig {
            rootfs: PathBuf::from("/rootfs"),
            command: "/bin/sh".to_string(),
            args: vec![],
            env: vec![],
            hostname: "test".to_string(),
            cgroup_path: PathBuf::from("/cgroup"),
            capture_output: false,
            hooks: ContainerHooks::default(),
            skip_network_namespace: false,
            mounts: vec![],    // placeholder — Task 6 replaces this
            privileged: false, // placeholder — Task 6 replaces this
        };

        let result = runtime.spawn_process(&config).await;
        assert!(result.is_ok(), "Runtime must spawn process successfully");

        let pid = result.unwrap().pid;
        assert!(pid > 0, "Runtime must return valid PID (> 0)");
    }

    #[tokio::test]
    async fn runtime_must_increment_pids_for_multiple_spawns() {
        let runtime = MockRuntime::new();
        let config = ContainerSpawnConfig {
            rootfs: PathBuf::from("/rootfs"),
            command: "/bin/sh".to_string(),
            args: vec![],
            env: vec![],
            hostname: "test".to_string(),
            cgroup_path: PathBuf::from("/cgroup"),
            capture_output: false,
            hooks: ContainerHooks::default(),
            skip_network_namespace: false,
            mounts: vec![],    // placeholder — Task 6 replaces this
            privileged: false, // placeholder — Task 6 replaces this
        };

        let pid1 = runtime.spawn_process(&config).await.unwrap().pid;
        let pid2 = runtime.spawn_process(&config).await.unwrap().pid;

        assert_ne!(pid1, pid2, "Runtime must return unique PIDs");
        assert!(pid2 > pid1, "Runtime PIDs should increment");
    }

    // -------------------------------------------------------------------------
    // Integration: Handler Conformance
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn handler_pull_must_work_with_any_registry_adapter() {
        let temp_dir = TempDir::new().unwrap();
        let deps = mock_deps(&temp_dir);
        let state = mock_state(&temp_dir);

        let response = handler::handle_pull(
            "alpine".to_string(),
            Some("latest".to_string()),
            state,
            deps,
        )
        .await;

        assert!(
            matches!(response, DaemonResponse::Success { .. }),
            "Pull handler must work with any ImageRegistry implementation"
        );
    }

    #[tokio::test]
    async fn handler_run_must_work_with_any_adapter_set() {
        let temp_dir = TempDir::new().unwrap();
        let deps = mock_deps_with_registry(
            MockRegistry::new().with_cached_image("library/alpine", "latest"),
            &temp_dir,
        );
        let state = mock_state(&temp_dir);

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

        assert!(
            matches!(response, DaemonResponse::ContainerCreated { .. }),
            "Run handler must work with any adapter set (Linux/WSL/Docker)"
        );
    }

    // -------------------------------------------------------------------------
    // NetworkProvider Conformance
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn network_noop_must_succeed_for_none_mode() {
        // NetworkMode::None is the default — setup must succeed and return a namespace path string.
        let net = MockNetwork::new();
        let config = NetworkConfig {
            mode: NetworkMode::None,
            ..NetworkConfig::default()
        };
        let result = net.setup("test-container-id", &config).await;
        assert!(result.is_ok(), "setup with NetworkMode::None must succeed");
    }

    #[tokio::test]
    async fn network_setup_must_return_namespace_path() {
        // setup() must return a non-empty string (the namespace path).
        let net = MockNetwork::new();
        let config = NetworkConfig {
            mode: NetworkMode::None,
            ..NetworkConfig::default()
        };
        let ns_path = net
            .setup("cid-abc", &config)
            .await
            .expect("setup must succeed");
        assert!(
            !ns_path.is_empty(),
            "setup must return a non-empty namespace path"
        );
    }

    #[tokio::test]
    async fn network_cleanup_must_succeed_after_setup() {
        // cleanup() must not return an error after a successful setup.
        let net = MockNetwork::new();
        let config = NetworkConfig {
            mode: NetworkMode::None,
            ..NetworkConfig::default()
        };
        net.setup("cid-xyz", &config)
            .await
            .expect("setup must succeed");
        let cleanup_result = net.cleanup("cid-xyz").await;
        assert!(cleanup_result.is_ok(), "cleanup must succeed after setup");
    }

    #[tokio::test]
    async fn handler_run_must_invoke_network_setup() {
        // Verifies that the handler wires NetworkProvider into the run path.
        // NOTE: handler_tests::test_network_setup_called_on_run covers this at the unit level.
        // This conformance test mirrors it using the conformance helper pattern to confirm
        // the same invariant holds for any adapter passed through HandlerDependencies.
        let temp_dir = TempDir::new().expect("create temp dir");
        let mock_network = std::sync::Arc::new(MockNetwork::new());
        let deps = mock_deps_with_network(mock_network.clone(), &temp_dir);
        let state = mock_state(&temp_dir);

        handle_run_once(
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

        assert_eq!(
            mock_network.setup_count(),
            1,
            "NetworkProvider::setup must be called exactly once per handle_run invocation"
        );
    }

    #[tokio::test]
    async fn handler_remove_must_work_with_any_filesystem_adapter() {
        let temp_dir = TempDir::new().unwrap();
        let deps = mock_deps_with_registry(
            MockRegistry::new().with_cached_image("library/alpine", "latest"),
            &temp_dir,
        );
        let state = mock_state(&temp_dir);

        // Create container first
        let create_response = handle_run_once(
            "alpine".to_string(),
            Some("latest".to_string()),
            vec!["/bin/sh".to_string()],
            None,
            None,
            false,
            state.clone(),
            deps.clone(),
        )
        .await;

        let container_id = match create_response {
            DaemonResponse::ContainerCreated { id } => id,
            _ => panic!("Expected ContainerCreated"),
        };

        // Wait and mark as stopped
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        state
            .update_container_state(&container_id, ContainerState::Stopped)
            .await
            .ok(); // container may already be Stopped (mock runtime exits immediately)

        // Remove
        let response = handler::handle_remove(container_id, state, deps).await;

        assert!(
            matches!(response, DaemonResponse::Success { .. }),
            "Remove handler must work with any FilesystemProvider implementation"
        );
    }
}

/// OS-specific behavior documentation tests.
///
/// These tests document expected differences between platforms
/// rather than asserting conformance.
#[cfg(test)]
mod platform_differences {
    #[test]
    #[ignore] // Documentation test
    fn linux_uses_native_overlayfs() {
        // Linux: Direct overlay mount syscall
        // WSL2: Delegated to WSL helper binary
        // Docker Desktop: Delegated to container in VM
    }

    #[test]
    #[ignore] // Documentation test
    fn wsl2_requires_path_translation() {
        // Windows paths (C:\...) must convert to WSL paths (/mnt/c/...)
        // Linux and Docker Desktop use paths directly
    }

    #[test]
    #[ignore] // Documentation test
    fn docker_desktop_uses_vm_networking() {
        // Docker Desktop: Operations run in LinuxKit VM
        // WSL2: Operations run in WSL2 VM
        // Linux: Operations run on host kernel directly
    }
}

/// Performance conformance tests.
///
/// Ensures adapters maintain acceptable performance characteristics.
#[cfg(test)]
mod performance_conformance {
    use super::*;

    #[tokio::test]
    async fn registry_has_image_must_complete_under_1ms() {
        let registry = MockRegistry::new().with_cached_image("alpine", "latest");
        let start = std::time::Instant::now();

        let _ = registry.has_image("alpine", "latest").await;

        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 1,
            "Registry has_image must complete under 1ms, took {elapsed:?}",
        );
    }

    #[test]
    fn filesystem_setup_must_complete_under_100ms() {
        let fs = MockFilesystem::new();
        let layers = vec![PathBuf::from("/layer1")];
        let container_dir = PathBuf::from("/container");

        let start = std::time::Instant::now();
        let _ = fs.setup_rootfs(&layers, &container_dir);
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 100,
            "Filesystem setup must complete under 100ms, took {elapsed:?}",
        );
    }

    #[test]
    fn limiter_create_must_complete_under_10ms() {
        let limiter = MockLimiter::new();
        let config = ResourceConfig::default();

        let start = std::time::Instant::now();
        let _ = limiter.create("container-123", &config);
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 10,
            "Limiter create must complete under 10ms, took {elapsed:?}",
        );
    }

    #[tokio::test]
    async fn network_noop_setup_must_complete_under_1ms() {
        // Performance: MockNetwork (no-op) setup must be near-instant.
        use std::time::Instant;
        let net = MockNetwork::new();
        let config = NetworkConfig {
            mode: NetworkMode::None,
            ..NetworkConfig::default()
        };
        let start = Instant::now();
        net.setup("perf-cid", &config)
            .await
            .expect("setup must succeed");
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 1,
            "noop network setup must complete in under 1ms, took {elapsed:?}",
        );
    }
}

// ---------------------------------------------------------------------------
// Issue #67 — Commit conformance tests
// ---------------------------------------------------------------------------

/// Conformance tests for the `ContainerCommitter` port (commit capability).
///
/// Uses `BackendDescriptor` from `minibox_core::adapters::conformance` and
/// `MockContainerCommitter` from `minibox_core::adapters::mocks`.
/// Tests are skipped automatically when `BackendCapability::Commit` is absent.
#[cfg(test)]
mod commit_conformance {
    use minibox_core::adapters::conformance::BackendDescriptor;
    use minibox_core::adapters::mocks::MockContainerCommitter;
    use minibox_core::domain::{BackendCapability, CommitConfig, ContainerId};
    use std::sync::Arc;

    fn mock_backend() -> BackendDescriptor {
        let committer = Arc::new(MockContainerCommitter::new());
        BackendDescriptor::new("mock-commit-backend").with_committer(move || {
            Arc::clone(&committer) as minibox_core::domain::DynContainerCommitter
        })
    }

    fn no_commit_backend() -> BackendDescriptor {
        BackendDescriptor::new("no-commit-backend")
    }

    // #67 — skip if !supports_commit
    #[tokio::test]
    async fn commit_is_skipped_when_capability_absent() {
        let descriptor = no_commit_backend();
        if !descriptor.capabilities.supports(BackendCapability::Commit) {
            // correct: capability absent, test should skip
            return;
        }
        panic!("expected no Commit capability for no_commit_backend");
    }

    // #67 — commit success path: mock returns ImageMetadata, verify fields match
    #[tokio::test]
    async fn commit_success_returns_matching_metadata() {
        let descriptor = mock_backend();
        if !descriptor.capabilities.supports(BackendCapability::Commit) {
            return;
        }
        let committer = descriptor.make_committer.as_ref().unwrap()();
        let container_id = ContainerId::new("testcontainer01".to_string()).unwrap();
        let config = CommitConfig {
            author: Some("conformance-test".to_string()),
            message: Some("commit test".to_string()),
            env_overrides: vec![],
            cmd_override: None,
        };
        let meta = committer
            .commit(&container_id, "conformance-image:v1", &config)
            .await
            .expect("commit must succeed");

        assert_eq!(meta.name, "conformance-image", "name must match target ref");
        assert_eq!(meta.tag, "v1", "tag must match target ref");
        assert!(
            !meta.layers.is_empty(),
            "committed image must have at least one layer"
        );
    }

    // #67 — backend-consistent: two fresh committers return structurally identical metadata
    #[tokio::test]
    async fn commit_two_backends_return_structurally_identical_metadata() {
        let descriptor_a = mock_backend();
        let descriptor_b = mock_backend();

        if !descriptor_a
            .capabilities
            .supports(BackendCapability::Commit)
            || !descriptor_b
                .capabilities
                .supports(BackendCapability::Commit)
        {
            return;
        }

        let committer_a = descriptor_a.make_committer.as_ref().unwrap()();
        let committer_b = descriptor_b.make_committer.as_ref().unwrap()();
        let container_id = ContainerId::new("testcontainer02".to_string()).unwrap();
        let config = CommitConfig {
            author: None,
            message: None,
            env_overrides: vec![],
            cmd_override: None,
        };

        let meta_a = committer_a
            .commit(&container_id, "image:latest", &config)
            .await
            .expect("commit A must succeed");
        let meta_b = committer_b
            .commit(&container_id, "image:latest", &config)
            .await
            .expect("commit B must succeed");

        assert_eq!(
            meta_a.name, meta_b.name,
            "both backends must return same image name"
        );
        assert_eq!(
            meta_a.tag, meta_b.tag,
            "both backends must return same image tag"
        );
        assert_eq!(
            meta_a.layers.len(),
            meta_b.layers.len(),
            "both backends must return same layer count"
        );
    }
}

// ---------------------------------------------------------------------------
// Issue #71 — Build conformance tests
// ---------------------------------------------------------------------------

/// Conformance tests for the `ImageBuilder` port (build capability).
///
/// Uses `BuildContextFixture` for a minimal Dockerfile build context and
/// `MockImageBuilder` for the in-memory adapter under test.
#[cfg(test)]
mod build_conformance {
    use minibox_core::adapters::conformance::{BackendDescriptor, BuildContextFixture};
    use minibox_core::adapters::mocks::MockImageBuilder;
    use minibox_core::domain::{BackendCapability, BuildConfig, BuildContext};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn mock_backend() -> BackendDescriptor {
        let builder = Arc::new(MockImageBuilder::new());
        BackendDescriptor::new("mock-build-backend")
            .with_builder(move || Arc::clone(&builder) as minibox_core::domain::DynImageBuilder)
    }

    fn no_build_backend() -> BackendDescriptor {
        BackendDescriptor::new("no-build-backend")
    }

    // #71 — skip if !supports_build
    #[tokio::test]
    async fn build_is_skipped_when_capability_absent() {
        let descriptor = no_build_backend();
        if !descriptor
            .capabilities
            .supports(BackendCapability::BuildFromContext)
        {
            return;
        }
        panic!("expected no BuildFromContext capability for no_build_backend");
    }

    // #71 — minimal Dockerfile build succeeds
    #[tokio::test]
    async fn build_minimal_dockerfile_succeeds() {
        let descriptor = mock_backend();
        if !descriptor
            .capabilities
            .supports(BackendCapability::BuildFromContext)
        {
            return;
        }

        let ctx_fixture = BuildContextFixture::new().expect("build context fixture");
        let builder = descriptor.make_builder.as_ref().unwrap()();

        let context = BuildContext {
            directory: ctx_fixture.context_dir.clone(),
            dockerfile: ctx_fixture.dockerfile.file_name().unwrap().into(),
        };
        let config = BuildConfig {
            tag: "conformance-build:latest".to_string(),
            build_args: vec![],
            no_cache: false,
        };

        let (tx, _rx) = mpsc::channel(16);
        let meta = builder
            .build_image(&context, &config, tx)
            .await
            .expect("build must succeed");

        assert_eq!(meta.name, "conformance-build");
        assert_eq!(meta.tag, "latest");
    }

    // #71 — metadata preservation: name/tag/digest survive the build
    #[tokio::test]
    async fn build_preserves_name_and_tag_through_build() {
        let descriptor = mock_backend();
        if !descriptor
            .capabilities
            .supports(BackendCapability::BuildFromContext)
        {
            return;
        }

        let ctx_fixture = BuildContextFixture::new().expect("build context fixture");
        let builder = descriptor.make_builder.as_ref().unwrap()();

        let context = BuildContext {
            directory: ctx_fixture.context_dir.clone(),
            dockerfile: ctx_fixture.dockerfile.file_name().unwrap().into(),
        };
        let config = BuildConfig {
            tag: "myapp:v2".to_string(),
            build_args: vec![],
            no_cache: false,
        };

        let (tx, _rx) = mpsc::channel(16);
        let meta = builder
            .build_image(&context, &config, tx)
            .await
            .expect("build must succeed");

        assert_eq!(meta.name, "myapp", "name must be preserved from tag");
        assert_eq!(meta.tag, "v2", "tag must be preserved from tag");
        assert!(
            !meta.layers.is_empty(),
            "built image must contain at least one layer"
        );
    }
}

// ---------------------------------------------------------------------------
// Issue #62 — Push conformance tests
// ---------------------------------------------------------------------------

/// Conformance tests for the `ImagePusher` port (push capability).
///
/// Uses `LocalPushTargetFixture` for reference strings and `MockImagePusher`
/// for the in-memory adapter that records pushes without a real registry.
#[cfg(test)]
mod push_conformance {
    use minibox_core::adapters::conformance::{BackendDescriptor, LocalPushTargetFixture};
    use minibox_core::adapters::mocks::MockImagePusher;
    use minibox_core::domain::{BackendCapability, RegistryCredentials};
    use minibox_core::image::reference::ImageRef;
    use std::sync::Arc;

    fn mock_backend() -> (BackendDescriptor, Arc<MockImagePusher>) {
        let pusher = Arc::new(MockImagePusher::new());
        let pusher_clone = Arc::clone(&pusher);
        let descriptor = BackendDescriptor::new("mock-push-backend")
            .with_pusher(move || Arc::clone(&pusher_clone) as minibox_core::domain::DynImagePusher);
        (descriptor, pusher)
    }

    fn no_push_backend() -> BackendDescriptor {
        BackendDescriptor::new("no-push-backend")
    }

    // #62 — skip if !supports_push
    #[tokio::test]
    async fn push_is_skipped_when_capability_absent() {
        let descriptor = no_push_backend();
        if !descriptor
            .capabilities
            .supports(BackendCapability::PushToRegistry)
        {
            return;
        }
        panic!("expected no PushToRegistry capability for no_push_backend");
    }

    // #62 — local test-registry push: mock returns a digest
    #[tokio::test]
    async fn push_to_mock_registry_returns_digest() {
        let (descriptor, _pusher) = mock_backend();
        if !descriptor
            .capabilities
            .supports(BackendCapability::PushToRegistry)
        {
            return;
        }

        let fixture = LocalPushTargetFixture::new("conformance/push-test");
        let image_ref = ImageRef::parse(&fixture.image_ref).expect("valid image ref");
        let pusher = descriptor.make_pusher.as_ref().unwrap()();

        let result = pusher
            .push_image(&image_ref, &RegistryCredentials::Anonymous, None)
            .await
            .expect("push must succeed");

        assert!(
            !result.digest.is_empty(),
            "push must return a non-empty digest"
        );
        assert!(
            result.digest.starts_with("sha256:"),
            "digest must use sha256 prefix"
        );
    }

    // #62 — reported digest matches what was sent
    #[tokio::test]
    async fn push_reported_digest_matches_image_content() {
        let (descriptor, pusher_handle) = mock_backend();
        if !descriptor
            .capabilities
            .supports(BackendCapability::PushToRegistry)
        {
            return;
        }

        let fixture = LocalPushTargetFixture::new("conformance/push-digest");
        let image_ref = ImageRef::parse(&fixture.image_ref).expect("valid image ref");
        let pusher = descriptor.make_pusher.as_ref().unwrap()();

        let result = pusher
            .push_image(&image_ref, &RegistryCredentials::Anonymous, None)
            .await
            .expect("push must succeed");

        // The mock records digests; verify the push was recorded
        let last_digest = pusher_handle.last_pushed_digest();
        assert_eq!(
            Some(result.digest.as_str()),
            last_digest.as_deref(),
            "reported digest must match what was recorded"
        );
    }

    // #62 — visible tags: after push, mock registry reports the tag as present
    #[tokio::test]
    async fn push_makes_tag_visible_in_mock_registry() {
        let (descriptor, pusher_handle) = mock_backend();
        if !descriptor
            .capabilities
            .supports(BackendCapability::PushToRegistry)
        {
            return;
        }

        let fixture = LocalPushTargetFixture::new("conformance/tag-visibility");
        let image_ref = ImageRef::parse(&fixture.image_ref).expect("valid image ref");
        let pusher = descriptor.make_pusher.as_ref().unwrap()();

        pusher
            .push_image(&image_ref, &RegistryCredentials::Anonymous, None)
            .await
            .expect("push must succeed");

        assert!(
            pusher_handle.has_tag(&fixture.image_ref),
            "mock registry must report tag as present after push"
        );
    }
}
