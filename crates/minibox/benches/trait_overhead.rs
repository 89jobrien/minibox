//! Benchmarks for hexagonal architecture trait overhead.
//!
//! Measures the performance impact of dynamic dispatch (trait objects)
//! compared to direct calls, validating that the architectural benefits
//! come at negligible runtime cost.
//!
//! Run with: `cargo bench --bench trait_overhead`

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use minibox::daemon::handler;
use minibox::daemon::handler::run::RunParams;
use minibox::daemon::state::{CgroupFreezeChecker, ProcessChecker};
use minibox::testing::helpers::{
    make_mock_deps, make_mock_state, make_mock_state_with_n_containers,
};
use minibox_core::adapters::mocks::{MockFilesystem, MockLimiter, MockRegistry, MockRuntime};
use minibox_core::domain::{
    ContainerHooks, ContainerRuntime, ContainerSpawnConfig, FilesystemProvider, ImageRegistry,
    ResourceConfig, ResourceLimiter, RootfsSetup,
};
use minibox_core::events::NoopEventSink;
use minibox_core::path::InternalPath;
use minibox_core::protocol::DaemonResponse;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::runtime::Runtime;

// ---------------------------------------------------------------------------
// Direct vs Trait Object Calls
// ---------------------------------------------------------------------------

fn bench_registry_direct_call(c: &mut Criterion) {
    c.bench_function("registry_direct_has_image", |b| {
        let registry = MockRegistry::new().with_cached_image("alpine", "latest");
        let rt = Runtime::new().unwrap();

        b.iter(|| rt.block_on(async { black_box(registry.has_image("alpine", "latest")).await }));
    });
}

fn bench_registry_trait_object_call(c: &mut Criterion) {
    c.bench_function("registry_trait_object_has_image", |b| {
        let registry: Arc<dyn ImageRegistry> =
            Arc::new(MockRegistry::new().with_cached_image("alpine", "latest"));
        let rt = Runtime::new().unwrap();

        b.iter(|| rt.block_on(async { black_box(registry.has_image("alpine", "latest")).await }));
    });
}

fn bench_filesystem_direct_call(c: &mut Criterion) {
    c.bench_function("filesystem_direct_setup", |b| {
        let fs = MockFilesystem::new();
        let layers = vec![PathBuf::from("/layer1")];
        let container_dir = PathBuf::from("/container");

        b.iter(|| black_box(fs.setup_rootfs(&layers, &container_dir)).ok());
    });
}

fn bench_filesystem_trait_object_call(c: &mut Criterion) {
    c.bench_function("filesystem_trait_object_setup", |b| {
        let fs: Arc<dyn FilesystemProvider> = Arc::new(MockFilesystem::new());
        let layers = vec![PathBuf::from("/layer1")];
        let container_dir = PathBuf::from("/container");

        b.iter(|| black_box(fs.setup_rootfs(&layers, &container_dir)).ok());
    });
}

fn bench_limiter_direct_call(c: &mut Criterion) {
    c.bench_function("limiter_direct_create", |b| {
        let limiter = MockLimiter::new();
        let config = ResourceConfig::default();

        b.iter(|| black_box(limiter.create("container-123", &config)).ok());
    });
}

fn bench_limiter_trait_object_call(c: &mut Criterion) {
    c.bench_function("limiter_trait_object_create", |b| {
        let limiter: Arc<dyn ResourceLimiter> = Arc::new(MockLimiter::new());
        let config = ResourceConfig::default();

        b.iter(|| black_box(limiter.create("container-123", &config)).ok());
    });
}

fn bench_runtime_direct_call(c: &mut Criterion) {
    c.bench_function("runtime_direct_spawn", |b| {
        let runtime = MockRuntime::new();
        let rt = Runtime::new().unwrap();
        let config = ContainerSpawnConfig {
            rootfs: InternalPath::from("/rootfs"),
            command: "/bin/sh".to_string(),
            args: vec![],
            env: vec![],
            hostname: "test".to_string(),
            cgroup_path: InternalPath::from("/cgroup"),
            capture_output: false,
            hooks: ContainerHooks::default(),
            skip_network_namespace: false,
            mounts: vec![],    // placeholder — Task 6 replaces this
            privileged: false, // placeholder — Task 6 replaces this
            image_ref: None,
        };

        b.iter(|| rt.block_on(async { black_box(runtime.spawn_process(&config).await).ok() }));
    });
}

fn bench_runtime_trait_object_call(c: &mut Criterion) {
    c.bench_function("runtime_trait_object_spawn", |b| {
        let runtime: Arc<dyn ContainerRuntime> = Arc::new(MockRuntime::new());
        let rt = Runtime::new().unwrap();
        let config = ContainerSpawnConfig {
            rootfs: InternalPath::from("/rootfs"),
            command: "/bin/sh".to_string(),
            args: vec![],
            env: vec![],
            hostname: "test".to_string(),
            cgroup_path: InternalPath::from("/cgroup"),
            capture_output: false,
            hooks: ContainerHooks::default(),
            skip_network_namespace: false,
            mounts: vec![],    // placeholder — Task 6 replaces this
            privileged: false, // placeholder — Task 6 replaces this
            image_ref: None,
        };

        b.iter(|| rt.block_on(async { black_box(runtime.spawn_process(&config).await).ok() }));
    });
}

// ---------------------------------------------------------------------------
// Arc Cloning Overhead
// ---------------------------------------------------------------------------

fn bench_arc_clone(c: &mut Criterion) {
    c.bench_function("arc_clone", |b| {
        let registry: Arc<dyn ImageRegistry> = Arc::new(MockRegistry::new());

        b.iter(|| black_box(Arc::clone(&registry)));
    });
}

// ---------------------------------------------------------------------------
// Downcasting Overhead
// ---------------------------------------------------------------------------

fn bench_downcast(c: &mut Criterion) {
    c.bench_function("downcast_to_concrete", |b| {
        let registry: Arc<dyn ImageRegistry> = Arc::new(MockRegistry::new());

        b.iter(|| black_box(registry.as_any().downcast_ref::<MockRegistry>()));
    });
}

criterion_group!(
    trait_overhead,
    bench_registry_direct_call,
    bench_registry_trait_object_call,
    bench_filesystem_direct_call,
    bench_filesystem_trait_object_call,
    bench_limiter_direct_call,
    bench_limiter_trait_object_call,
    bench_runtime_direct_call,
    bench_runtime_trait_object_call,
    bench_arc_clone,
    bench_downcast,
);

// ---------------------------------------------------------------------------
// DaemonState::reconcile_on_startup — startup scan at scale
// ---------------------------------------------------------------------------

/// Process checker that never calls kill(2) — safe in bench context.
struct AlwaysDead;
impl ProcessChecker for AlwaysDead {
    fn is_alive(&self, _pid: u32) -> bool {
        false
    }
}

struct NeverFrozen;
impl CgroupFreezeChecker for NeverFrozen {
    fn is_frozen(&self, _cgroup_path: &std::path::Path) -> bool {
        false
    }
}

fn bench_state_reconcile(c: &mut Criterion) {
    let mut group = c.benchmark_group("state_reconcile");

    for n in [10usize, 100, 500] {
        // TempDir must outlive the benchmark group — create it per size.
        let tmp = TempDir::new().expect("temp dir for reconcile bench");
        let state = make_mock_state_with_n_containers(tmp.path(), n);
        let rt = Runtime::new().expect("tokio runtime");

        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                rt.block_on(async {
                    state.reconcile_on_startup(&AlwaysDead, &NeverFrozen).await;
                    black_box(())
                })
            });
        });
        // Keep tmp alive until after bench_with_input completes.
        drop(tmp);
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Handler pipeline — async dispatch through mocks
// ---------------------------------------------------------------------------

fn bench_handler_list(c: &mut Criterion) {
    let mut group = c.benchmark_group("handler_pipeline_list");
    let rt = Runtime::new().expect("tokio runtime");

    for n in [0usize, 10, 100] {
        let tmp = TempDir::new().expect("temp dir");
        let state = make_mock_state_with_n_containers(tmp.path(), n);

        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| rt.block_on(async { black_box(handler::handle_list(state.clone()).await) }));
        });
        drop(tmp);
    }

    group.finish();
}

/// Benchmark full `handle_run` dispatch with mock adapters (image-miss path).
///
/// The mock registry has no cached image so the handler returns
/// `DaemonResponse::Error` after the registry look-up fails.  This path still
/// exercises: policy gate → image-presence check → channel send.
fn bench_handler_run_dispatch(c: &mut Criterion) {
    let tmp = TempDir::new().expect("temp dir for handler_run bench");
    let deps = make_mock_deps(&tmp);
    let state = make_mock_state(tmp.path());
    let rt = Runtime::new().expect("tokio runtime");

    c.bench_function("handler_pipeline_run_mock_image_miss", |b| {
        b.iter(|| {
            rt.block_on(async {
                let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
                let params = RunParams {
                    image: "alpine".to_string(),
                    tag: None,
                    command: vec!["/bin/sh".to_string()],
                    memory_limit_bytes: None,
                    cpu_weight: None,
                    ephemeral: false,
                    network: None,
                    mounts: vec![],
                    privileged: false,
                    env: vec![],
                    name: None,
                    platform: None,
                    cgroup_parent: None,
                    priority: None,
                    policy_override: None,
                };
                handler::handle_run(params, state.clone(), deps.clone(), tx).await;
                black_box(rx.recv().await)
            })
        });
    });
}

/// Benchmark `handle_pause` on a non-existent container (fast-path error return).
///
/// Measures: async RwLock acquire + HashMap lookup + channel-less response.
fn bench_handler_pause_not_found(c: &mut Criterion) {
    let tmp = TempDir::new().expect("temp dir");
    let state = make_mock_state(tmp.path());
    let event_sink: Arc<dyn minibox_core::events::EventSink> = Arc::new(NoopEventSink);
    let rt = Runtime::new().expect("tokio runtime");

    c.bench_function("handler_pipeline_pause_not_found", |b| {
        b.iter(|| {
            rt.block_on(async {
                black_box(
                    handler::handle_pause(
                        "nonexistent-bench-id".to_string(),
                        state.clone(),
                        event_sink.clone(),
                    )
                    .await,
                )
            })
        });
    });
}

criterion_group!(state_reconcile, bench_state_reconcile);
criterion_group!(
    handler_pipeline,
    bench_handler_list,
    bench_handler_run_dispatch,
    bench_handler_pause_not_found
);
criterion_main!(trait_overhead, state_reconcile, handler_pipeline);
