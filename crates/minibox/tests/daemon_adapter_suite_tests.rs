//! Integration tests for adapter suite dispatch paths: native, gke, colima.
//!
//! These tests exercise the full handler → adapter → response path for each
//! adapter suite using mock or in-process adapters, so no root privileges or
//! external processes are required.
//!
//! # Test strategy
//!
//! Each test wires `HandlerDependencies` with the adapters that correspond to
//! the named suite (mock variants for platform adapters not available on the
//! host), then calls `handle_run` and asserts the expected `DaemonResponse`.
//!
//! - **native**: gated `#[cfg(target_os = "linux")]`; uses `MockRuntime`,
//!   `MockFilesystem`, `MockLimiter`, and `MockNetwork` — all the same
//!   leaf-adapter types that the real native suite uses, just without real
//!   syscalls.
//! - **gke**: uses `MockRegistry` + `MockRuntime` + `CopyFilesystem` +
//!   `NoopLimiter` + `NoopNetwork`.
//! - **colima**: uses `MockRegistry` + `ColimaRuntime::with_executor` +
//!   `ColimaFilesystem` + `ColimaLimiter` + `NoopNetwork`.

use minibox::adapters::mocks::{MockFilesystem, MockRegistry, MockRuntime};
use minibox::daemon::handler::{
    self, BuildDeps, EventDeps, ExecDeps, HandlerDependencies, ImageDeps, LifecycleDeps,
    NoopImageLoader,
};
use minibox::daemon::state::DaemonState;
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::DynImageRegistry;
use minibox_core::image::ImageStore;
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

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

/// Drive `handle_run` through a one-shot channel and return the first message.
#[allow(clippy::too_many_arguments)]
async fn handle_run_once(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        image,
        tag,
        command,
        None, // memory_limit_bytes
        None, // cpu_weight
        false,
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

fn make_state(tmp: &TempDir) -> Arc<DaemonState> {
    let image_store = ImageStore::new(tmp.path().join("images")).unwrap();
    Arc::new(DaemonState::new(image_store, tmp.path()))
}

fn make_deps_from_parts(
    registry: Arc<MockRegistry>,
    filesystem: impl minibox_core::domain::FilesystemProvider + 'static,
    resource_limiter: impl minibox_core::domain::ResourceLimiter + 'static,
    runtime: impl minibox_core::domain::ContainerRuntime + 'static,
    network: impl minibox_core::domain::NetworkProvider + 'static,
    tmp: &TempDir,
) -> Arc<HandlerDependencies> {
    let image_store = Arc::new(ImageStore::new(tmp.path().join("images2")).unwrap());
    Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                registry as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(filesystem),
            resource_limiter: Arc::new(resource_limiter),
            runtime: Arc::new(runtime),
            network_provider: Arc::new(network),
            containers_base: tmp.path().join("containers"),
            run_containers_base: tmp.path().join("run"),
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
            event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
            metrics: Arc::new(minibox::daemon::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: minibox::daemon::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: false,
        },
        checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
    })
}

// ---------------------------------------------------------------------------
// Native adapter suite (Linux only)
// ---------------------------------------------------------------------------

/// Exercises the handler dispatch path with native-equivalent mock adapters.
///
/// On the real native suite these would be `OverlayFilesystem`, `CgroupV2Limiter`,
/// `LinuxNamespaceRuntime`, and `BridgeNetwork`/`NoopNetwork`. Here we use
/// mock doubles that implement the same traits — verifying that the handler
/// routes correctly through the full `RunContainer` → `ContainerCreated` path.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn test_native_adapter_suite_run_returns_container_created() {
    let tmp = TempDir::new().expect("tempdir");
    let registry = Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest"));
    let deps = make_deps_from_parts(
        registry,
        MockFilesystem::new(),
        MockLimiter::new(),
        MockRuntime::new(),
        MockNetwork::new(),
        &tmp,
    );
    let state = make_state(&tmp);

    let response = handle_run_once(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec!["/bin/sh".to_string()],
        state,
        deps,
    )
    .await;

    match response {
        DaemonResponse::ContainerCreated { id } => {
            assert!(!id.is_empty(), "container id must be non-empty");
        }
        other => panic!("native suite: expected ContainerCreated, got {other:?}"),
    }
}

/// Native adapter suite: registry pull failure propagates to DaemonResponse::Error.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn test_native_adapter_suite_pull_failure_returns_error() {
    let tmp = TempDir::new().expect("tempdir");
    let registry = Arc::new(MockRegistry::new().with_pull_failure());
    let deps = make_deps_from_parts(
        registry,
        MockFilesystem::new(),
        MockLimiter::new(),
        MockRuntime::new(),
        MockNetwork::new(),
        &tmp,
    );
    let state = make_state(&tmp);

    let response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        state,
        deps,
    )
    .await;

    assert!(
        matches!(response, DaemonResponse::Error { .. }),
        "native suite: expected Error on pull failure, got {response:?}"
    );
}

// ---------------------------------------------------------------------------
// GKE adapter suite
// ---------------------------------------------------------------------------

/// GKE suite wires `CopyFilesystem`, `NoopLimiter`, and a stub runtime.
///
/// `ProotRuntime` requires the `proot` binary at runtime — not available in
/// CI. We use `MockRuntime` instead to isolate handler logic from process
/// spawning, which is the correct unit boundary.
#[tokio::test]
async fn test_gke_adapter_suite_run_returns_container_created() {
    use minibox::adapters::{CopyFilesystem, NoopLimiter, NoopNetwork};

    let tmp = TempDir::new().expect("tempdir");
    let registry = Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest"));
    let deps = make_deps_from_parts(
        registry,
        CopyFilesystem::new(),
        NoopLimiter::new(),
        MockRuntime::new(), // stands in for ProotRuntime (requires proot binary)
        NoopNetwork::new(),
        &tmp,
    );
    let state = make_state(&tmp);

    let response = handle_run_once(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec!["/bin/sh".to_string()],
        state,
        deps,
    )
    .await;

    match response {
        DaemonResponse::ContainerCreated { id } => {
            assert!(!id.is_empty(), "gke suite: container id must be non-empty");
        }
        other => panic!("gke suite: expected ContainerCreated, got {other:?}"),
    }
}

/// GKE suite: registry pull failure propagates to DaemonResponse::Error.
#[tokio::test]
async fn test_gke_adapter_suite_pull_failure_returns_error() {
    use minibox::adapters::{CopyFilesystem, NoopLimiter, NoopNetwork};

    let tmp = TempDir::new().expect("tempdir");
    let registry = Arc::new(MockRegistry::new().with_pull_failure());
    let deps = make_deps_from_parts(
        registry,
        CopyFilesystem::new(),
        NoopLimiter::new(),
        MockRuntime::new(),
        NoopNetwork::new(),
        &tmp,
    );
    let state = make_state(&tmp);

    let response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        state,
        deps,
    )
    .await;

    assert!(
        matches!(response, DaemonResponse::Error { .. }),
        "gke suite: expected Error on pull failure, got {response:?}"
    );
}

// ---------------------------------------------------------------------------
// Colima adapter suite
// ---------------------------------------------------------------------------

/// Colima suite wires `ColimaRuntime` (with a fake executor so no `limactl`
/// binary is required), `ColimaLimiter`, and `NoopNetwork`.
///
/// `ColimaFilesystem` shells out to `limactl shell` — not available in CI.
/// We substitute `MockFilesystem` for the rootfs setup step so the test
/// exercises the Colima runtime dispatch path without spawning host processes.
#[tokio::test]
async fn test_colima_adapter_suite_run_returns_container_created() {
    use minibox::adapters::{ColimaLimiter, ColimaRuntime, NoopNetwork};

    let tmp = TempDir::new().expect("tempdir");
    let registry = Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest"));

    // Inject a fake executor so ColimaRuntime never shells out to limactl/nerdctl.
    let colima_runtime =
        ColimaRuntime::new().with_executor(Arc::new(|_args: &[&str]| Ok(String::new())));

    let deps = make_deps_from_parts(
        registry,
        MockFilesystem::new(), // ColimaFilesystem requires limactl; use mock for rootfs setup
        ColimaLimiter::new(),
        colima_runtime,
        NoopNetwork::new(),
        &tmp,
    );
    let state = make_state(&tmp);

    let response = handle_run_once(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec!["/bin/sh".to_string()],
        state,
        deps,
    )
    .await;

    match response {
        DaemonResponse::ContainerCreated { id } => {
            assert!(
                !id.is_empty(),
                "colima suite: container id must be non-empty"
            );
        }
        other => panic!("colima suite: expected ContainerCreated, got {other:?}"),
    }
}

/// Colima suite: registry pull failure propagates to DaemonResponse::Error.
#[tokio::test]
async fn test_colima_adapter_suite_pull_failure_returns_error() {
    use minibox::adapters::{ColimaLimiter, ColimaRuntime, NoopNetwork};

    let tmp = TempDir::new().expect("tempdir");
    let registry = Arc::new(MockRegistry::new().with_pull_failure());

    let colima_runtime =
        ColimaRuntime::new().with_executor(Arc::new(|_args: &[&str]| Ok(String::new())));

    let deps = make_deps_from_parts(
        registry,
        MockFilesystem::new(),
        ColimaLimiter::new(),
        colima_runtime,
        NoopNetwork::new(),
        &tmp,
    );
    let state = make_state(&tmp);

    let response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        state,
        deps,
    )
    .await;

    assert!(
        matches!(response, DaemonResponse::Error { .. }),
        "colima suite: expected Error on pull failure, got {response:?}"
    );
}
