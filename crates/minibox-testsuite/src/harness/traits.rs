//! `ConformanceTest` trait and supporting types.
//!
//! Every conformance test in this crate is a struct implementing `ConformanceTest`.
//! The trait is `Send + Sync` so the `TestRunner` can execute tests in parallel.

use serde::{Deserialize, Serialize};

use super::context::TestContext;

/// Broad category of a conformance test — used for filtering and reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestCategory {
    /// Verifies a single trait method or invariant in isolation.
    Unit,
    /// Verifies interactions between multiple trait implementations.
    Integration,
    /// Boundary conditions, empty inputs, and error paths.
    EdgeCase,
}

impl TestCategory {
    #[must_use]
    /// Returns the stable report label for this category.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unit => "unit",
            Self::Integration => "integration",
            Self::EdgeCase => "edge_case",
        }
    }
}

/// Outcome of a single conformance test run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum TestResult {
    /// Test completed successfully.
    Pass,
    /// Test failed an assertion or operation.
    Fail {
        /// Human-readable failure reason.
        reason: String,
    },
    /// Test could not run in the current environment.
    Skipped {
        /// Human-readable skip reason.
        reason: String,
    },
}

impl TestResult {
    #[must_use]
    /// Returns whether this is a passing result.
    pub const fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }
    #[must_use]
    /// Returns whether this is a failing result.
    pub const fn is_fail(&self) -> bool {
        matches!(self, Self::Fail { .. })
    }
    #[must_use]
    /// Returns whether this is a skipped result.
    pub const fn is_skipped(&self) -> bool {
        matches!(self, Self::Skipped { .. })
    }
}

/// Trait every conformance test must implement.
///
/// # Example
///
/// ```rust,ignore
/// use minibox_testsuite::harness::{ConformanceTest, TestCategory, TestContext, TestResult};
///
/// struct RegistryPullCountTest;
///
/// impl ConformanceTest for RegistryPullCountTest {
///     fn name(&self) -> &str { "registry_pull_increments_count" }
///     fn adapter(&self) -> &str { "registry" }
///     fn category(&self) -> TestCategory { TestCategory::Unit }
///
///     fn run_sync(&self, ctx: &mut TestContext<'_>) -> TestResult {
///         use minibox::testing::mocks::registry::MockRegistry;
///         use minibox_core::domain::ImageRegistry;
///
///         let registry = MockRegistry::new();
///         let image = minibox_core::image::reference::ImageRef::parse("alpine:3.18").unwrap();
///
///         // Drive async from sync context — conformance tests avoid a Tokio runtime dep.
///         let rt = tokio::runtime::Runtime::new().unwrap();
///         rt.block_on(registry.pull_image(&image)).unwrap();
///
///         ctx.assert_eq(1, registry.pull_count(), "pull_count after one pull");
///         ctx.result()
///     }
/// }
/// ```
pub trait ConformanceTest: Send + Sync {
    /// Short `snake_case` identifier unique within the adapter scope.
    fn name(&self) -> &str;

    /// Which adapter this test exercises: `"registry"`, `"runtime"`, `"filesystem"`, etc.
    fn adapter(&self) -> &str;

    /// Category used for filtering and report grouping.
    fn category(&self) -> TestCategory;

    /// Execute the test. Receives a fresh `TestContext`; returns the aggregate result.
    ///
    /// Implementations call `ctx.assert_*` methods to record pass/fail, then return
    /// `ctx.result()` at the end.
    /// Optional: declare required capability for auto-skip.
    /// Default: `None` (always runs).
    fn required_capability(&self) -> Option<minibox_core::domain::BackendCapability> {
        None
    }

    /// Executes the test synchronously using the supplied context.
    fn run_sync(&self, ctx: &mut TestContext<'_>) -> TestResult;

    /// Fully-qualified test id: `"<adapter>::<name>"`.
    fn id(&self) -> String {
        format!("{}::{}", self.adapter(), self.name())
    }
}
