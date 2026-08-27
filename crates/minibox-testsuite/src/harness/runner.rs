#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::unwrap_in_result,
    clippy::redundant_clone,
    clippy::redundant_closure,
    clippy::single_char_pattern,
    clippy::redundant_field_names,
    clippy::uninlined_format_args,
    clippy::manual_assert,
    clippy::duration_suboptimal_units,
    clippy::unnecessary_map_or,
    clippy::unnecessary_literal_bound
)]
//! `TestRunner` — collects and executes `ConformanceTest` instances.
//!
//! Tests run sequentially (parallelism is intentionally omitted — the mock
//! adapters are stateful and cheap enough that parallel execution adds no
//! meaningful benefit while complicating output ordering).

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use minibox_core::adapters::conformance::BackendDescriptor;

use super::context::TestContext;
use super::macros::ConformanceTestEntry;
use super::traits::{ConformanceTest, TestCategory, TestResult};

/// Result of a single test execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestRunResult {
    /// Fully-qualified `"adapter::name"` id.
    pub id: String,
    /// Short test name.
    pub name: String,
    /// Adapter category exercised by the test.
    pub adapter: String,
    /// Broad test category.
    pub category: TestCategory,
    /// Test outcome.
    #[serde(flatten)]
    pub result: TestResult,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Failure reasons (empty on pass/skip).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<String>,
}

/// Aggregate summary of a runner execution.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestSummary {
    /// Total number of executed and skipped tests.
    pub total: usize,
    /// Number of passing tests.
    pub passed: usize,
    /// Number of failing tests.
    pub failed: usize,
    /// Number of skipped tests.
    pub skipped: usize,
    /// Total runner duration in milliseconds.
    pub duration_ms: u64,
    /// Individual test results in execution order.
    pub results: Vec<TestRunResult>,
}

/// Paths to reports emitted for one conformance execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConformanceArtifacts {
    /// Machine-readable JSON report.
    pub json: PathBuf,
    /// JUnit XML report for CI ingestion.
    pub junit: PathBuf,
    /// Human-readable Markdown report.
    pub markdown: PathBuf,
}

/// Aggregate counters and timing for one conformance execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConformanceSummary {
    /// Total number of executed and skipped tests.
    pub total: usize,
    /// Number of passing tests.
    pub passed: usize,
    /// Number of failing tests.
    pub failed: usize,
    /// Number of skipped tests.
    pub skipped: usize,
    /// Total runner duration in milliseconds.
    pub duration_ms: u64,
    /// Whether the execution completed without test failures.
    pub success: bool,
}

/// Typed outcome of a complete conformance execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConformanceResult {
    /// Report schema version.
    pub report_version: String,
    /// UTC timestamp at which the result was assembled.
    pub generated_at: String,
    /// Aggregate counters and execution timing.
    pub summary: ConformanceSummary,
    /// Per-test outcomes, timing, and assertion diagnostics in execution order.
    #[serde(rename = "results")]
    pub outcomes: Vec<TestRunResult>,
    /// Paths to all generated report artifacts.
    pub artifacts: ConformanceArtifacts,
}

impl ConformanceResult {
    /// Combines a runner summary with its generated report paths.
    #[must_use]
    pub fn new(summary: &TestSummary, artifacts: ConformanceArtifacts) -> Self {
        Self {
            report_version: "1.0".to_string(),
            generated_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            summary: ConformanceSummary {
                total: summary.total,
                passed: summary.passed,
                failed: summary.failed,
                skipped: summary.skipped,
                duration_ms: summary.duration_ms,
                success: summary.is_success(),
            },
            outcomes: summary.results.clone(),
            artifacts,
        }
    }

    /// Returns whether the execution completed without test failures.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.summary.success
    }
}

impl TestSummary {
    #[must_use]
    /// Returns whether the run completed without failures.
    pub const fn is_success(&self) -> bool {
        self.failed == 0
    }

    /// Results grouped by adapter name.
    #[must_use]
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
    descriptor: Option<BackendDescriptor>,
}

impl Default for TestRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl TestRunner {
    #[must_use]
    /// Creates an empty test runner with no filters.
    pub fn new() -> Self {
        Self {
            tests: Vec::new(),
            filter: RunnerFilter::default(),
            descriptor: None,
        }
    }

    /// Collect all tests registered via `inventory`.
    #[must_use]
    pub fn collect_inventory() -> Self {
        let tests: Vec<Box<dyn ConformanceTest>> = inventory::iter::<ConformanceTestEntry>
            .into_iter()
            .map(|entry| (entry.make)())
            .collect();
        Self {
            tests,
            filter: RunnerFilter::default(),
            descriptor: None,
        }
    }

    /// Set the backend descriptor for capability-based auto-skip.
    #[must_use]
    pub fn with_descriptor(mut self, desc: BackendDescriptor) -> Self {
        self.descriptor = Some(desc);
        self
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
    #[must_use]
    pub fn filter_adapter(mut self, name: &str) -> Self {
        self.filter.adapter = Some(name.to_string());
        self
    }

    /// Filter to a specific category.
    #[must_use]
    pub const fn filter_category(mut self, cat: TestCategory) -> Self {
        self.filter.category = Some(cat);
        self
    }

    /// Filter by name substring.
    #[must_use]
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
    #[must_use]
    pub fn count(&self) -> usize {
        self.tests.len()
    }

    /// Number of tests that will execute after filtering.
    #[must_use]
    pub fn filtered_count(&self) -> usize {
        self.tests
            .iter()
            .filter(|t| self.passes_filter(t.as_ref()))
            .count()
    }

    /// Execute all (filtered) tests and return the summary.
    #[must_use]
    pub fn run(&self) -> TestSummary {
        let suite_start = Instant::now();
        let mut results = Vec::new();

        for test in &self.tests {
            if !self.passes_filter(test.as_ref()) {
                continue;
            }

            // Auto-skip if required capability is not supported.
            if let Some(cap) = test.required_capability() {
                if let Some(ref desc) = self.descriptor {
                    if !desc.capabilities.supports(cap) {
                        results.push(TestRunResult {
                            id: test.id(),
                            name: test.name().to_string(),
                            adapter: test.adapter().to_string(),
                            category: test.category(),
                            result: TestResult::Skipped {
                                reason: format!("backend does not support {cap:?}"),
                            },
                            duration_ms: 0,
                            failures: Vec::new(),
                        });
                        continue;
                    }
                }
            }

            let start = Instant::now();
            let mut ctx = match &self.descriptor {
                Some(desc) => TestContext::with_descriptor(desc),
                None => TestContext::new(),
            };
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
    use std::path::PathBuf;

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
        fn run_sync(&self, ctx: &mut TestContext<'_>) -> TestResult {
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
        fn run_sync(&self, ctx: &mut TestContext<'_>) -> TestResult {
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
    fn inventory_collects_expected_test_count() {
        // Pin the floor so a dropped adapter module or a stripped inventory
        // ctor section cannot silently zero the suite. Update EXPECTED_MIN
        // when adding or removing conformance suites.
        const EXPECTED_MIN: usize = 123;
        let runner = TestRunner::collect_inventory();
        assert!(
            runner.count() >= EXPECTED_MIN,
            "conformance inventory collapsed: {} tests (expected >= {EXPECTED_MIN})",
            runner.count()
        );
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

    #[test]
    fn conformance_result_round_trips_outcomes_artifacts_and_timing() {
        let mut runner = TestRunner::new();
        runner.add(PassTest);
        runner.add(FailTest);
        let summary = runner.run();
        let artifacts = ConformanceArtifacts {
            json: PathBuf::from("artifacts/conformance/conformance.json"),
            junit: PathBuf::from("artifacts/conformance/conformance.xml"),
            markdown: PathBuf::from("artifacts/conformance/conformance.md"),
        };
        let result = ConformanceResult::new(&summary, artifacts);

        let encoded = serde_json::to_string(&result).expect("serialize conformance result");
        let decoded: ConformanceResult =
            serde_json::from_str(&encoded).expect("deserialize conformance result");

        assert_eq!(decoded.summary.total, 2);
        assert_eq!(decoded.summary.passed, 1);
        assert_eq!(decoded.summary.failed, 1);
        assert_eq!(decoded.outcomes.len(), 2);
        assert_eq!(decoded.summary.duration_ms, result.summary.duration_ms);
        assert_eq!(decoded.artifacts, result.artifacts);
        assert!(decoded.outcomes[1].result.is_fail());
        assert_eq!(decoded.report_version, "1.0");
        assert!(!decoded.generated_at.is_empty());
    }
}
