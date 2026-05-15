//! `minibox-testsuite` — conformance test harness for minibox adapter contracts.
//!
//! # Structure
//!
//! ```text
//! minibox-testsuite/
//!   src/
//!     harness/          ← ConformanceTest trait, TestContext, TestRunner, ReportGenerator
//!     adapters/         ← per-adapter test modules (registry, runtime, limiter, state)
//!     bin/
//!       run_conformance.rs    ← CLI: run all tests, exit 1 on failure
//!       generate_report.rs    ← CLI: run tests, write JSON + JUnit reports to artifacts/
//! ```
//!
//! # Usage
//!
//! Run the full suite:
//!
//! ```bash
//! cargo run -p minibox-testsuite --bin run-conformance
//! ```
//!
//! Generate machine-readable reports:
//!
//! ```bash
//! cargo run -p minibox-testsuite --bin generate-report
//! ```
//!
//! Both binaries exit `0` on success, `1` on any test failure.

pub mod adapters;
pub mod harness;

/// Convenience re-export of the full harness prelude.
pub mod prelude {
    pub use crate::harness::{
        ConformanceTest, ReportConfig, ReportGenerator, TestCategory, TestContext, TestResult,
        TestRunner, TestSummary,
    };
}
