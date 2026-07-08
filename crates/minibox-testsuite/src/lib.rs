//! `minibox-testsuite` — conformance test harness for minibox adapter contracts.
// Test infrastructure: expect/unwrap calls are appropriate for assertion failures.
#![allow(clippy::expect_used, clippy::unwrap_used)]
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

/// Declare a conformance test with inventory auto-registration.
///
/// # With capability (auto-skip when unsupported)
///
/// ```rust,ignore
/// conformance_test! {
///     name: "commit_roundtrip",
///     adapter: "container_committer",
///     capability: Commit,
///     category: Unit,
///     |ctx| {
///         ctx.result()
///     }
/// }
/// ```
///
/// # Without capability (always runs)
///
/// ```rust,ignore
/// conformance_test! {
///     name: "pull_increments_count",
///     adapter: "registry",
///     category: Unit,
///     |ctx| {
///         ctx.result()
///     }
/// }
/// ```
#[macro_export]
macro_rules! conformance_test {
    // Variant WITH capability
    (
        name: $name:expr,
        adapter: $adapter:expr,
        capability: $cap:ident,
        category: $cat:ident,
        |$ctx:ident| $body:block
    ) => {
        $crate::conformance_test!(@inner
            $name, $adapter,
            Some(minibox_core::domain::BackendCapability::$cap),
            $cat, $ctx, $body
        );
    };
    // Variant WITHOUT capability
    (
        name: $name:expr,
        adapter: $adapter:expr,
        category: $cat:ident,
        |$ctx:ident| $body:block
    ) => {
        $crate::conformance_test!(@inner
            $name, $adapter,
            None,
            $cat, $ctx, $body
        );
    };
    // Internal expansion
    (@inner
        $name:expr, $adapter:expr,
        $cap:expr,
        $cat:ident, $ctx:ident, $body:block
    ) => {
        paste::paste! {
            #[allow(non_camel_case_types)]
            struct [< __Conformance_ $adapter:camel _ $name:camel >];

            impl $crate::harness::ConformanceTest
                for [< __Conformance_ $adapter:camel _ $name:camel >]
            {
                fn name(&self) -> &str { $name }
                fn adapter(&self) -> &str { $adapter }
                fn category(&self) -> $crate::harness::TestCategory {
                    $crate::harness::TestCategory::$cat
                }
                fn required_capability(&self)
                    -> Option<minibox_core::domain::BackendCapability>
                {
                    $cap
                }
                fn run_sync(
                    &self,
                    $ctx: &mut $crate::harness::TestContext<'_>,
                ) -> $crate::harness::TestResult
                $body
            }

            inventory::submit! {
                $crate::harness::ConformanceTestEntry {
                    make: || -> Box<dyn $crate::harness::ConformanceTest> {
                        Box::new([< __Conformance_ $adapter:camel _ $name:camel >])
                    },
                }
            }
        }
    };
}

/// Convenience re-export of the full harness prelude.
pub mod prelude {
    pub use crate::harness::{
        ConformanceTest, ReportConfig, ReportGenerator, TestCategory, TestContext, TestResult,
        TestRunner, TestSummary,
    };
}
