//! Benchmarks for hexagonal architecture trait overhead.
//!
//! Measures the performance impact of dynamic dispatch (trait objects)
//! compared to direct calls, validating that the architectural benefits
//! come at negligible runtime cost.
//!
//! Run with: `cargo bench -p minibox-bench --bench trait_dispatch`

// Bench setup code: panicking on setup failure is the correct behaviour.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use criterion::{Criterion, criterion_main};
use minibox_core::adapters::mocks::{MockFilesystem, MockLimiter, MockRegistry, MockRuntime};
use minibox_core::domain::{
    ContainerHooks, ContainerRuntime, ContainerSpawnConfig, FilesystemProvider, ImageRegistry,
    ResourceConfig, ResourceLimiter, RootfsSetup,
};
use minibox_core::path::InternalPath;
use std::hint::black_box;
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

fn spawn_config() -> ContainerSpawnConfig {
    ContainerSpawnConfig {
        rootfs: InternalPath::from("/rootfs"),
        command: "/bin/sh".to_string(),
        args: vec![],
        env: vec![],
        hostname: "test".to_string(),
        cgroup_path: InternalPath::from("/cgroup"),
        capture_output: false,
        hooks: ContainerHooks::default(),
        skip_network_namespace: false,
        mounts: vec![],
        privileged: false,
        image_ref: None,
    }
}

fn bench_runtime_direct_call(c: &mut Criterion) {
    c.bench_function("runtime_direct_spawn", |b| {
        let runtime = MockRuntime::new();
        let rt = Runtime::new().unwrap();
        let config = spawn_config();

        b.iter(|| rt.block_on(async { black_box(runtime.spawn_process(&config).await).ok() }));
    });
}

fn bench_runtime_trait_object_call(c: &mut Criterion) {
    c.bench_function("runtime_trait_object_spawn", |b| {
        let runtime: Arc<dyn ContainerRuntime> = Arc::new(MockRuntime::new());
        let rt = Runtime::new().unwrap();
        let config = spawn_config();

        b.iter(|| rt.block_on(async { black_box(runtime.spawn_process(&config).await).ok() }));
    });
}

minibox_bench::documented_criterion_group!(
    "Runs adapter trait dispatch benchmarks.",
    trait_dispatch,
    bench_registry_direct_call,
    bench_registry_trait_object_call,
    bench_filesystem_direct_call,
    bench_filesystem_trait_object_call,
    bench_limiter_direct_call,
    bench_limiter_trait_object_call,
    bench_runtime_direct_call,
    bench_runtime_trait_object_call,
);
criterion_main!(trait_dispatch);
