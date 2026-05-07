//! `TestContext` — the unified interface passed into every conformance test.
//!
//! Tracks assertion failures, captures structured log output, and returns the
//! aggregate `TestResult` at the end of a test via `ctx.result()`.

use std::fmt::Debug;

use super::traits::TestResult;

/// Structured log entry captured during a test run.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub kind: LogKind,
    pub label: String,
    pub value: String,
}

/// Classification of a log entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogKind {
    Input,
    Expected,
    Actual,
    Info,
    Fail,
}

/// Context object threaded through every conformance test.
///
/// Collects assertion failures and structured log output. At the end of a test,
/// call `ctx.result()` to obtain the aggregate `TestResult`.
pub struct TestContext {
    failures: Vec<String>,
    log: Vec<LogEntry>,
}

impl Default for TestContext {
    fn default() -> Self {
        Self::new()
    }
}

impl TestContext {
    /// Create a fresh context with no recorded state.
    pub fn new() -> Self {
        Self {
            failures: Vec::new(),
            log: Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Logging helpers
    // -----------------------------------------------------------------------

    /// Record a test input for diagnostic output.
    pub fn log_input<T: Debug>(&mut self, label: &str, value: &T) {
        self.log.push(LogEntry {
            kind: LogKind::Input,
            label: label.to_string(),
            value: format!("{value:?}"),
        });
    }

    /// Record an expected value (from a fixture or specification).
    pub fn log_expected<T: Debug>(&mut self, label: &str, value: &T) {
        self.log.push(LogEntry {
            kind: LogKind::Expected,
            label: label.to_string(),
            value: format!("{value:?}"),
        });
    }

    /// Record the actual value produced by the implementation under test.
    pub fn log_actual<T: Debug>(&mut self, label: &str, value: &T) {
        self.log.push(LogEntry {
            kind: LogKind::Actual,
            label: label.to_string(),
            value: format!("{value:?}"),
        });
    }

    /// Log a free-form informational note.
    pub fn log_info(&mut self, msg: &str) {
        self.log.push(LogEntry {
            kind: LogKind::Info,
            label: String::new(),
            value: msg.to_string(),
        });
    }

    /// Return all log entries accumulated so far.
    pub fn log_entries(&self) -> &[LogEntry] {
        &self.log
    }

    // -----------------------------------------------------------------------
    // Assertion helpers
    // -----------------------------------------------------------------------

    /// Assert `actual == expected` using `PartialEq`. Records a failure if not equal.
    ///
    /// Returns `true` on pass, `false` on fail.
    pub fn assert_eq<T: PartialEq + Debug>(&mut self, expected: T, actual: T, label: &str) -> bool {
        if expected == actual {
            true
        } else {
            let reason = format!("{label}: expected {expected:?}, got {actual:?}");
            self.record_failure(reason);
            false
        }
    }

    /// Assert `actual != forbidden`. Records a failure if equal.
    pub fn assert_ne<T: PartialEq + Debug>(
        &mut self,
        forbidden: T,
        actual: T,
        label: &str,
    ) -> bool {
        if forbidden != actual {
            true
        } else {
            self.record_failure(format!(
                "{label}: expected value != {forbidden:?}, but got equal"
            ));
            false
        }
    }

    /// Assert `condition` is true. Records a failure with `label` as the reason.
    pub fn assert_true(&mut self, condition: bool, label: &str) -> bool {
        if condition {
            true
        } else {
            self.record_failure(format!("{label}: expected true, got false"));
            false
        }
    }

    /// Assert `condition` is false.
    pub fn assert_false(&mut self, condition: bool, label: &str) -> bool {
        if !condition {
            true
        } else {
            self.record_failure(format!("{label}: expected false, got true"));
            false
        }
    }

    /// Assert a `Result` is `Ok`. Records the error as a failure if `Err`.
    ///
    /// Returns `Some(value)` on `Ok`, `None` on `Err`.
    pub fn assert_ok<T, E: Debug>(&mut self, result: Result<T, E>, label: &str) -> Option<T> {
        match result {
            Ok(v) => Some(v),
            Err(e) => {
                self.record_failure(format!("{label}: expected Ok, got Err({e:?})"));
                None
            }
        }
    }

    /// Assert a `Result` is `Err`. Records a failure if `Ok`.
    pub fn assert_err<T, E>(&mut self, result: Result<T, E>, label: &str) -> bool {
        match result {
            Err(_) => true,
            Ok(_) => {
                self.record_failure(format!("{label}: expected Err, got Ok"));
                false
            }
        }
    }

    /// Assert a string contains a substring.
    pub fn assert_contains(&mut self, haystack: &str, needle: &str, label: &str) -> bool {
        if haystack.contains(needle) {
            true
        } else {
            self.record_failure(format!(
                "{label}: expected {haystack:?} to contain {needle:?}"
            ));
            false
        }
    }

    // -----------------------------------------------------------------------
    // Failure tracking
    // -----------------------------------------------------------------------

    /// Manually record a failure with a descriptive reason.
    pub fn record_failure(&mut self, reason: String) {
        self.log.push(LogEntry {
            kind: LogKind::Fail,
            label: String::new(),
            value: reason.clone(),
        });
        self.failures.push(reason);
    }

    /// Returns `true` if any assertion has failed.
    pub fn has_failures(&self) -> bool {
        !self.failures.is_empty()
    }

    /// All failure reasons accumulated so far.
    pub fn failures(&self) -> &[String] {
        &self.failures
    }

    /// Consume the context and return the aggregate `TestResult`.
    ///
    /// Call this as the last line of `ConformanceTest::run_sync`.
    pub fn result(&self) -> TestResult {
        if self.failures.is_empty() {
            TestResult::Pass
        } else {
            TestResult::Fail {
                reason: self.failures.join("; "),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_context_has_no_failures() {
        let ctx = TestContext::new();
        assert!(!ctx.has_failures());
        assert!(ctx.result().is_pass());
    }

    #[test]
    fn assert_eq_pass() {
        let mut ctx = TestContext::new();
        assert!(ctx.assert_eq(42, 42, "value"));
        assert!(ctx.result().is_pass());
    }

    #[test]
    fn assert_eq_fail() {
        let mut ctx = TestContext::new();
        assert!(!ctx.assert_eq(42, 43, "value"));
        assert!(ctx.result().is_fail());
        assert!(ctx.failures()[0].contains("expected 42"));
    }

    #[test]
    fn assert_ok_returns_value() {
        let mut ctx = TestContext::new();
        let val = ctx.assert_ok(Ok::<i32, &str>(7), "parse");
        assert_eq!(val, Some(7));
        assert!(ctx.result().is_pass());
    }

    #[test]
    fn assert_ok_records_err() {
        let mut ctx = TestContext::new();
        let val = ctx.assert_ok(Err::<i32, &str>("oops"), "parse");
        assert!(val.is_none());
        assert!(ctx.result().is_fail());
    }

    #[test]
    fn multiple_failures_joined() {
        let mut ctx = TestContext::new();
        ctx.assert_eq(1, 2, "a");
        ctx.assert_eq(3, 4, "b");
        match ctx.result() {
            TestResult::Fail { reason } => {
                assert!(reason.contains("a:"));
                assert!(reason.contains("b:"));
            }
            _ => panic!("expected Fail"),
        }
    }
}
