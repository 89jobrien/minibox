//! Conformance tests for the `ContainerRuntime` trait contract.
//!
//! All tests use `MockRuntime` — no syscalls are made.

use minibox::testing::mocks::runtime::MockRuntime;
use minibox_core::domain::{ContainerHooks, ContainerRuntime, ContainerSpawnConfig};

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

fn default_config() -> ContainerSpawnConfig {
    ContainerSpawnConfig {
        rootfs: std::path::PathBuf::from("/mock/rootfs"),
        command: "/bin/sh".to_string(),
        args: vec![],
        env: vec![],
        hostname: "conformance-test".to_string(),
        cgroup_path: std::path::PathBuf::from("/mock/cgroup/conformanceruntime01"),
        capture_output: false,
        hooks: ContainerHooks::default(),
        skip_network_namespace: false,
        mounts: vec![],
        privileged: false,
    }
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

pub struct SpawnReturnsNonzeroPid;
impl ConformanceTest for SpawnReturnsNonzeroPid {
    fn name(&self) -> &str { "spawn_returns_nonzero_pid" }
    fn adapter(&self) -> &str { "runtime" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let runtime = MockRuntime::new();
        let handle = ctx.assert_ok(
            rt().block_on(runtime.spawn_process(&default_config())),
            "spawn_process succeeds",
        );
        if let Some(h) = handle {
            ctx.assert_true(h.pid > 0, "spawned PID > 0");
        }
        ctx.result()
    }
}

pub struct SpawnIncrementsCount;
impl ConformanceTest for SpawnIncrementsCount {
    fn name(&self) -> &str { "spawn_increments_count" }
    fn adapter(&self) -> &str { "runtime" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let runtime = MockRuntime::new();
        rt().block_on(runtime.spawn_process(&default_config())).expect("spawn");
        ctx.assert_eq(1, runtime.spawn_count(), "spawn_count after one spawn");
        ctx.result()
    }
}

pub struct SuccessiveSpawnsDifferentPids;
impl ConformanceTest for SuccessiveSpawnsDifferentPids {
    fn name(&self) -> &str { "successive_spawns_different_pids" }
    fn adapter(&self) -> &str { "runtime" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let runtime = MockRuntime::new();
        let cfg = default_config();
        let first = rt().block_on(runtime.spawn_process(&cfg)).expect("first spawn");
        let second = rt().block_on(runtime.spawn_process(&cfg)).expect("second spawn");
        ctx.assert_ne(first.pid, second.pid, "successive spawns get different PIDs");
        ctx.result()
    }
}

pub struct SpawnFailureReturnsErr;
impl ConformanceTest for SpawnFailureReturnsErr {
    fn name(&self) -> &str { "spawn_failure_returns_err" }
    fn adapter(&self) -> &str { "runtime" }
    fn category(&self) -> TestCategory { TestCategory::EdgeCase }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let runtime = MockRuntime::new().with_spawn_failure();
        ctx.assert_err(
            rt().block_on(runtime.spawn_process(&default_config())),
            "failure-configured mock returns Err",
        );
        ctx.result()
    }
}

pub struct SpawnFailureIncrementsCount;
impl ConformanceTest for SpawnFailureIncrementsCount {
    fn name(&self) -> &str { "spawn_failure_increments_count" }
    fn adapter(&self) -> &str { "runtime" }
    fn category(&self) -> TestCategory { TestCategory::EdgeCase }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let runtime = MockRuntime::new().with_spawn_failure();
        let _ = rt().block_on(runtime.spawn_process(&default_config()));
        ctx.assert_eq(1, runtime.spawn_count(), "failed spawn still counted");
        ctx.result()
    }
}

pub struct CapabilitiesReturnsStruct;
impl ConformanceTest for CapabilitiesReturnsStruct {
    fn name(&self) -> &str { "capabilities_returns_struct" }
    fn adapter(&self) -> &str { "runtime" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let runtime = MockRuntime::new();
        // Must not panic; values are not asserted — mock declares none.
        let _caps = runtime.capabilities();
        ctx.assert_true(true, "capabilities() does not panic");
        ctx.result()
    }
}

pub struct SyncAsyncConsistency;
impl ConformanceTest for SyncAsyncConsistency {
    fn name(&self) -> &str { "sync_async_consistency" }
    fn adapter(&self) -> &str { "runtime" }
    fn category(&self) -> TestCategory { TestCategory::Integration }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let runtime = MockRuntime::new();
        let cfg = default_config();
        let async_h = ctx.assert_ok(
            rt().block_on(runtime.spawn_process(&cfg)),
            "async spawn succeeds",
        );
        let sync_h = ctx.assert_ok(runtime.spawn_process_sync(&cfg), "sync spawn succeeds");
        if let (Some(a), Some(s)) = (async_h, sync_h) {
            ctx.assert_true(a.pid > 0, "async PID > 0");
            ctx.assert_true(s.pid > 0, "sync PID > 0");
            ctx.assert_ne(a.pid, s.pid, "async and sync return different PIDs");
        }
        ctx.assert_eq(2, runtime.spawn_count(), "both spawns counted");
        ctx.result()
    }
}

pub struct SyncAsyncFailureConsistency;
impl ConformanceTest for SyncAsyncFailureConsistency {
    fn name(&self) -> &str { "sync_async_failure_consistency" }
    fn adapter(&self) -> &str { "runtime" }
    fn category(&self) -> TestCategory { TestCategory::EdgeCase }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let runtime = MockRuntime::new().with_spawn_failure();
        let cfg = default_config();
        ctx.assert_err(rt().block_on(runtime.spawn_process(&cfg)), "async Err on failure");
        ctx.assert_err(runtime.spawn_process_sync(&cfg), "sync Err on failure");
        ctx.assert_eq(2, runtime.spawn_count(), "both failed spawns counted");
        ctx.result()
    }
}

/// Return all runtime conformance tests.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(SpawnReturnsNonzeroPid),
        Box::new(SpawnIncrementsCount),
        Box::new(SuccessiveSpawnsDifferentPids),
        Box::new(SpawnFailureReturnsErr),
        Box::new(SpawnFailureIncrementsCount),
        Box::new(CapabilitiesReturnsStruct),
        Box::new(SyncAsyncConsistency),
        Box::new(SyncAsyncFailureConsistency),
    ]
}
