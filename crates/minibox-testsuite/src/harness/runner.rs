//! `TestRunner` — collects and executes `ConformanceTest` instances.
//!
//! Tests run sequentially (parallelism is intentionally omitted — the mock
//! adapters are stateful and cheap enough that parallel execution adds no
//! meaningful benefit while complicating output ordering).

use std::collections::HashMap;
use std::time::Instant;

use serde::Serialize;

use super::context::TestContext;
use super::traits::{ConformanceTest, TestCategory, TestResult};

/// Result of a single test execution.
#[derive(Debug, Clone, Serialize)]
pub struct TestRunResult {
    /// Fully-qualified `"adapter::name"` id.
    pub id: String,
    pub name: String,
    pub adapter: String,
    pub category: TestCategory,
    #[serde(flatten)]
    pub result: TestResult,
    pub duration_ms: u64,
    /// Failure reasons (empty on pass/skip).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<String>,
}

/// Aggregate summary of a runner execution.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TestSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub duration_ms: u64,
    pub results: Vec<TestRunResult>,
}

impl TestSummary {
    pub fn is_success(&self) -> bool {
        self.failed == 0
    }

    /// Results grouped by adapter name.
    pub fn by_adapter(&self) -> HashMap<&str, Vec<&TestRunResult>> {
        let mut map: HashMap<&str, Vec<&TestRunResult>> = HashMap::new();
        for r in &self.results {
            map.entry(r.adapter.as_str()).or_default().push(r);
        }
        map
    }
}

/// Optional filters applied before running.
#[derive(Debug, Default)]
pub struct RunnerFilter {
    /// Only run tests for this adapter (exact match).
    pub adapter: Option<String>,
    /// Only run tests of this category.
    pub category: Option<TestCategory>,
    /// Only run tests whose name contains this substring.
    pub name_pattern: Option<String>,
}

/// Collects `ConformanceTest` instances and runs them.
pub struct TestRunner {
    tests: Vec<Box<dyn ConformanceTest>>,
    filter: RunnerFilter,
}

impl Default for TestRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl TestRunner {
    pub fn new() -> Self {
        Self {
            tests: Vec::new(),
            filter: RunnerFilter::default(),
        }
    }

    /// Register a single test.
    pub fn add<T: ConformanceTest + 'static>(&mut self, test: T) {
        self.tests.push(Box::new(test));
    }

    /// Register all tests from an iterator of `Box<dyn ConformanceTest>`.
    pub fn add_all(&mut self, tests: impl IntoIterator<Item = Box<dyn ConformanceTest>>) {
        self.tests.extend(tests);
    }

    /// Filter to a specific adapter.
    pub fn filter_adapter(mut self, name: &str) -> Self {
        self.filter.adapter = Some(name.to_string());
        self
    }

    /// Filter to a specific category.
    pub fn filter_category(mut self, cat: TestCategory) -> Self {
        self.filter.category = Some(cat);
        self
    }

    /// Filter by name substring.
    pub fn filter_name(mut self, pattern: &str) -> Self {
        self.filter.name_pattern = Some(pattern.to_string());
        self
    }

    fn passes_filter(&self, t: &dyn ConformanceTest) -> bool {
        if let Some(ref a) = self.filter.adapter {
            if t.adapter() != a {
                return false;
            }
        }
        if let Some(cat) = self.filter.category {
            if t.category() != cat {
                return false;
            }
        }
        if let Some(ref pat) = self.filter.name_pattern {
            if !t.name().contains(pat.as_str()) {
                return false;
            }
        }
        true
    }

    /// Number of registered tests.
    pub fn count(&self) -> usize {
        self.tests.len()
    }

    /// Number of tests that will execute after filtering.
    pub fn filtered_count(&self) -> usize {
        self.tests
            .iter()
            .filter(|t| self.passes_filter(t.as_ref()))
            .count()
    }

    /// Execute all (filtered) tests and return the summary.
    pub fn run(&self) -> TestSummary {
        let suite_start = Instant::now();
        let mut results = Vec::new();

        for test in &self.tests {
            if !self.passes_filter(test.as_ref()) {
                continue;
            }

            let start = Instant::now();
            let mut ctx = TestContext::new();
            let result = test.run_sync(&mut ctx);
            let duration_ms = start.elapsed().as_millis() as u64;

            let failures = ctx.failures().to_vec();
            results.push(TestRunResult {
                id: test.id(),
                name: test.name().to_string(),
                adapter: test.adapter().to_string(),
                category: test.category(),
                result,
                duration_ms,
                failures,
            });
        }

        let mut summary = TestSummary {
            duration_ms: suite_start.elapsed().as_millis() as u64,
            ..Default::default()
        };

        for r in &results {
            summary.total += 1;
            match &r.result {
                TestResult::Pass => summary.passed += 1,
                TestResult::Fail { .. } => summary.failed += 1,
                TestResult::Skipped { .. } => summary.skipped += 1,
            }
        }

        summary.results = results;
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PassTest;
    impl ConformanceTest for PassTest {
        fn name(&self) -> &str {
            "pass"
        }
        fn adapter(&self) -> &str {
            "mock"
        }
        fn category(&self) -> TestCategory {
            TestCategory::Unit
        }
        fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
            ctx.assert_eq(1, 1, "always");
            ctx.result()
        }
    }

    struct FailTest;
    impl ConformanceTest for FailTest {
        fn name(&self) -> &str {
            "fail"
        }
        fn adapter(&self) -> &str {
            "mock"
        }
        fn category(&self) -> TestCategory {
            TestCategory::Unit
        }
        fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
            ctx.assert_eq(1, 2, "mismatch");
            ctx.result()
        }
    }

    #[test]
    fn empty_runner_passes() {
        let runner = TestRunner::new();
        let summary = runner.run();
        assert_eq!(summary.total, 0);
        assert!(summary.is_success());
    }

    #[test]
    fn passing_test_recorded() {
        let mut runner = TestRunner::new();
        runner.add(PassTest);
        let summary = runner.run();
        assert_eq!(summary.total, 1);
        assert_eq!(summary.passed, 1);
        assert!(summary.is_success());
    }

    #[test]
    fn failing_test_recorded() {
        let mut runner = TestRunner::new();
        runner.add(FailTest);
        let summary = runner.run();
        assert_eq!(summary.failed, 1);
        assert!(!summary.is_success());
    }

    #[test]
    fn filter_by_name_works() {
        let mut runner = TestRunner::new();
        runner.add(PassTest);
        runner.add(FailTest);
        let runner = runner.filter_name("pass");
        assert_eq!(runner.filtered_count(), 1);
        let summary = runner.run();
        assert_eq!(summary.total, 1);
        assert_eq!(summary.passed, 1);
    }
}
