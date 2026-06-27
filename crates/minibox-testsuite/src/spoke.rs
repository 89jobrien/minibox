//! Spoke registration for external crate conformance tests.
//!
//! Spoke crates call [`SpokeRegistry::register`] to contribute tests to the
//! central conformance runner without being compiled into `minibox-testsuite`.
//!
//! # Hub + Spokes model
//!
//! `minibox-testsuite` is the **hub**: it owns the `TestRunner`, harness types,
//! and the built-in adapter modules. External crates (adapter implementations,
//! integration test helpers) are **spokes**: they call `register()` to inject
//! their tests into the hub at runtime.
//!
//! ```rust,ignore
//! use minibox_testsuite::spoke::SpokeRegistry;
//!
//! let mut spokes = SpokeRegistry::new();
//! spokes.register(my_crate::conformance::all());
//!
//! let all_tests = spokes.into_tests();
//! runner.add_all(all_tests);
//! ```

use crate::harness::ConformanceTest;

/// Collects conformance tests from spoke crates.
///
/// Spoke crates register their `Vec<Box<dyn ConformanceTest>>` batches here
/// and the hub runner consumes the collected tests via [`into_tests`].
///
/// [`into_tests`]: SpokeRegistry::into_tests
#[derive(Default)]
pub struct SpokeRegistry {
    tests: Vec<Box<dyn ConformanceTest>>,
}

impl SpokeRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a batch of tests from a spoke crate.
    ///
    /// May be called multiple times; tests accumulate in registration order.
    pub fn register(&mut self, tests: Vec<Box<dyn ConformanceTest>>) {
        self.tests.extend(tests);
    }

    /// Consume the registry and return all registered tests.
    #[must_use]
    pub fn into_tests(self) -> Vec<Box<dyn ConformanceTest>> {
        self.tests
    }

    /// Number of registered tests.
    #[must_use]
    pub fn count(&self) -> usize {
        self.tests.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{TestCategory, TestContext, TestResult};

    struct AlwaysPass;
    impl ConformanceTest for AlwaysPass {
        fn name(&self) -> &str {
            "always_pass"
        }
        fn adapter(&self) -> &str {
            "spoke_test"
        }
        fn category(&self) -> TestCategory {
            TestCategory::Unit
        }
        fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
            ctx.result()
        }
    }

    #[test]
    fn empty_registry_has_zero_count() {
        let reg = SpokeRegistry::new();
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn register_accumulates_tests() {
        let mut reg = SpokeRegistry::new();
        reg.register(vec![Box::new(AlwaysPass)]);
        reg.register(vec![Box::new(AlwaysPass), Box::new(AlwaysPass)]);
        assert_eq!(reg.count(), 3);
    }

    #[test]
    fn into_tests_returns_all() {
        let mut reg = SpokeRegistry::new();
        reg.register(vec![Box::new(AlwaysPass), Box::new(AlwaysPass)]);
        let tests = reg.into_tests();
        assert_eq!(tests.len(), 2);
    }

    #[test]
    fn spoke_tests_merge_into_runner() {
        use crate::harness::TestRunner;

        let mut reg = SpokeRegistry::new();
        reg.register(vec![Box::new(AlwaysPass), Box::new(AlwaysPass)]);

        let mut runner = TestRunner::new();
        runner.add_all(reg.into_tests());
        assert_eq!(runner.count(), 2);

        let summary = runner.run();
        assert_eq!(summary.passed, 2);
        assert!(summary.is_success());
    }
}
