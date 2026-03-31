//! Benchmarks for hexagonal architecture trait overhead.
//!
//! Measures the performance impact of dynamic dispatch (trait objects)
//! compared to direct calls, validating that the architectural benefits
//! come at negligible runtime cost.
//!
//! Run with: `cargo bench --bench trait_overhead`

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use mbx::adapters::mocks::{MockFilesystem, MockLimiter, MockRegistry, MockRuntime};
use mbx::domain::{
    ContainerHooks, ContainerRuntime, ContainerSpawnConfig, FilesystemProvider, ImageRegistry,
    ResourceConfig, ResourceLimiter,
};
use std::path::PathBuf;
use std::sync::Arc;
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

        b.iter(|| rt.block_on(async { black_box(runtime.spawn_process(&config).await).ok() }));
    });
}

fn bench_runtime_trait_object_call(c: &mut Criterion) {
    c.bench_function("runtime_trait_object_spawn", |b| {
        let runtime: Arc<dyn ContainerRuntime> = Arc::new(MockRuntime::new());
        let rt = Runtime::new().unwrap();
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

criterion_main!(trait_overhead);
