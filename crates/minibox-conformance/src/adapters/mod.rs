//! Per-adapter conformance test modules.
//!
//! Each module exposes an `all()` function returning `Vec<Box<dyn ConformanceTest>>`.
//! The `run-conformance` binary collects all adapters and feeds them to `TestRunner`.

pub mod limiter;
pub mod pause_resume;
pub mod registry;
pub mod runtime;
pub mod state;

use crate::harness::ConformanceTest;

/// Collect every conformance test across all adapters.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    let mut tests: Vec<Box<dyn ConformanceTest>> = Vec::new();
    tests.extend(registry::all());
    tests.extend(runtime::all());
    tests.extend(limiter::all());
    tests.extend(state::all());
    tests.extend(pause_resume::all());
    tests
}
