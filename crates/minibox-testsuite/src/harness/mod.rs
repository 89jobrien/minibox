//! Conformance harness: traits, context, runner, and report generator.

pub mod context;
pub mod macros;
pub mod report;
pub mod runner;
pub mod traits;

// Convenience re-exports for adapter modules.
pub use context::TestContext;
pub use macros::ConformanceTestEntry;
pub use report::{ReportConfig, ReportGenerator};
pub use runner::{
    ConformanceArtifacts, ConformanceResult, ConformanceSummary, TestRunResult, TestRunner,
    TestSummary,
};
pub use traits::{ConformanceTest, TestCategory, TestResult};
