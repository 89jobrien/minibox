//! Conformance harness: traits, context, runner, and report generator.

pub mod context;
pub mod report;
pub mod runner;
pub mod traits;

// Convenience re-exports for adapter modules.
pub use context::TestContext;
pub use report::{ReportConfig, ReportGenerator};
pub use runner::{TestRunner, TestSummary};
pub use traits::{ConformanceTest, TestCategory, TestResult};
