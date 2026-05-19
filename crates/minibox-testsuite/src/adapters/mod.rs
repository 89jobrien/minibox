//\! Per-adapter conformance test modules.
//\!
//\! Each module exposes an `all()` function returning `Vec<Box<dyn ConformanceTest>>`.
//\! The `run-conformance` binary collects all adapters and feeds them to `TestRunner`.

pub mod container_committer;
pub mod container_id;
pub mod exec_runtime;
pub mod filesystem;
pub mod image_builder;
pub mod image_loader;
pub mod image_pusher;
pub mod limiter;
pub mod list;
pub mod logs;
pub mod metrics;
pub mod network;
pub mod pause_resume;
pub mod policy;
pub mod pty;
pub mod registry;
pub mod registry_router;
pub mod runtime;
pub mod state;
pub mod vm_checkpoint;

use crate::harness::ConformanceTest;

/// Collect every conformance test across all adapters.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    let mut tests: Vec<Box<dyn ConformanceTest>> = Vec::new();
    tests.extend(registry::all());
    tests.extend(runtime::all());
    tests.extend(limiter::all());
    tests.extend(state::all());
    tests.extend(pause_resume::all());
    tests.extend(list::all());
    tests.extend(policy::all());
    tests.extend(container_id::all());
    tests.extend(logs::all());
    tests.extend(filesystem::all());
    tests.extend(exec_runtime::all());
    tests.extend(image_pusher::all());
    tests.extend(container_committer::all());
    tests.extend(image_builder::all());
    tests.extend(network::all());
    tests.extend(pty::all());
    tests.extend(vm_checkpoint::all());
    tests.extend(metrics::all());
    tests.extend(registry_router::all());
    tests.extend(image_loader::all());
    tests
}
