//! `Scenario` trait — the showcase-suite analogue of
//! `harness::ConformanceTest`.
//!
//! A `Scenario` drives real CLI/daemon interactions (as opposed to
//! `ConformanceTest`, which drives mock adapters in-process) and reports
//! progress through `&dyn Reporter` so the same code works for a narrated
//! demo binary and a silent assertion-mode `#[test]` fn.
//!
//! `ScenarioCtx` itself lives in [`super::context`] (it owns a real spawned
//! daemon + binary discovery, so it doesn't belong in this trait-only
//! module); it is re-exported from [`super`] for convenience.

use minibox_core::domain::BackendCapability;

use super::context::ScenarioCtx;
use super::reporter::Reporter;

/// A single showcase scenario: exercises one capability area end to end
/// (e.g. bridge networking + port forwarding) against a real daemon/CLI,
/// reporting progress via `&dyn Reporter`.
pub trait Scenario: Send + Sync {
    /// Short `snake_case` identifier, unique within the showcase suite.
    fn name(&self) -> &'static str;

    /// Capability this scenario needs from the active backend. `None`
    /// means it always runs (either because it works on every adapter, or
    /// because the gate can't be expressed by `BackendCapability` and is
    /// checked ad hoc inside `run()`). Default: `None`.
    fn required_capability(&self) -> Option<BackendCapability> {
        None
    }

    /// Execute the scenario. Implementations should call
    /// `Reporter::skip()` and return `Ok(())` early when a required
    /// capability, platform, or privilege is unavailable, rather than
    /// returning an `Err` — matching `ConformanceTest`'s
    /// skip-not-fail convention.
    fn run(&self, ctx: &ScenarioCtx, r: &dyn Reporter) -> anyhow::Result<()>;
}
