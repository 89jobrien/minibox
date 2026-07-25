//! Showcase harness: shared scenario-reporting abstractions for demo and
//! test consumers.
//!
//! A [`Scenario`] drives real CLI/daemon interactions end to end (as
//! opposed to `harness::ConformanceTest`, which drives mock adapters
//! in-process) and reports progress through `&dyn Reporter` so the same
//! scenario code works for both a narrated demo binary
//! (`cargo xtask demo`, via [`reporter::NarratedReporter`]) and a silent
//! assertion-mode `#[test]` fn (via [`reporter::SilentAssertReporter`]).
//!
//! # Structure
//!
//! - [`reporter`] — the `Reporter` trait plus its two implementations.
//! - [`scenario`] — the `Scenario` trait.
//! - [`context`] — `ScenarioCtx`: binary discovery, real daemon spawn/
//!   teardown, and `BackendDescriptor` capability gating.
//! - One module per showcase scenario ([`lifecycle`], [`pause_resume`],
//!   [`mounts_privileged`], [`networking`], [`build_commit_push`]), each
//!   implementing [`Scenario`] for a single capability area.

pub mod build_commit_push;
pub mod context;
pub mod lifecycle;
pub mod mounts_privileged;
pub mod networking;
pub mod pause_resume;
pub mod reporter;
pub mod scenario;

pub use context::ScenarioCtx;
pub use reporter::{NarratedReporter, Reporter, SilentAssertReporter};
pub use scenario::Scenario;
