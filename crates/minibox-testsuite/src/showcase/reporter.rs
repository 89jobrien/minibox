//! Reporter abstraction shared by narrated demos (`cargo xtask demo`) and
//! silent-assertion e2e/showcase tests.
//!
//! Scenario code is written once against `&dyn Reporter` and works
//! identically whether it's driving a human-narrated walkthrough or a
//! `#[test]` fn that should panic on failure. See
//! `crates/minibox-testsuite/src/harness/runner.rs` for the analogous
//! `TestResult::Skipped { reason }` self-skip convention this trait's
//! `skip()` method is meant to mirror.

use std::fmt;
use std::sync::Mutex;

/// Abstraction over "what to do when a scenario step happens".
///
/// Implemented once for human-narrated demo output and once for silent
/// assertion-mode tests, so scenario authors never branch on which mode
/// they're running in. `Send + Sync` because `build_commit_push`'s async
/// scenario code holds `&dyn Reporter` across `.await` points.
pub trait Reporter: Send + Sync {
    /// Announce the start of a named scenario (top-level section header in
    /// narrated mode; a no-op in test mode).
    fn section(&self, name: &str);

    /// Announce a discrete step about to run, e.g. "pulling alpine",
    /// "starting container", "checking memory.max cgroup file".
    fn step(&self, name: &str);

    /// Stream a line of live subprocess output associated with the current
    /// step. Narrated mode prints it; test mode buffers it for inclusion in
    /// failure messages.
    fn output(&self, line: &str);

    /// Report successful completion of the current step.
    fn success(&self, msg: &str);

    /// Report that a step/scenario was skipped and why (e.g. missing
    /// `BackendCapability`, non-root, non-Linux). Mirrors the
    /// `TestResult::Skipped { reason }` semantics used by
    /// `harness::runner::TestRunner` so assertion-mode tests self-skip
    /// rather than fail when a capability is absent.
    fn skip(&self, reason: &str);

    /// Report a hard failure for the current step. Narrated mode prints an
    /// error and moves on to the next scenario; assertion mode panics with
    /// the message plus any buffered context.
    fn failure(&self, msg: &str);

    /// Emitted once after all scenarios/steps have finished. No-op by
    /// default (assertion mode has no need for a cross-scenario summary).
    fn summary(&self) {}
}

/// Narrated mode: prints to stdout with headers/indentation for a human
/// audience running a demo binary.
pub struct NarratedReporter;

impl NarratedReporter {
    /// Create a new narrated reporter.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for NarratedReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl Reporter for NarratedReporter {
    fn section(&self, name: &str) {
        println!("\n=== {name} ===");
    }

    fn step(&self, name: &str) {
        println!("  -> {name}");
    }

    fn output(&self, line: &str) {
        println!("     | {line}");
    }

    fn success(&self, msg: &str) {
        println!("  [ok] {msg}");
    }

    fn skip(&self, reason: &str) {
        println!("  [skip] {reason}");
    }

    fn failure(&self, msg: &str) {
        eprintln!("  [FAIL] {msg}");
    }

    fn summary(&self) {
        println!("\nDemo complete.");
    }
}

/// Assertion mode: stays silent on the happy path.
///
/// Buffers step/output lines for failure diagnostics, and turns
/// `failure()` into a real panic with captured context — matching how
/// existing e2e tests assert on `CmdOutput { success, stdout, stderr }`
/// with descriptive panic messages.
pub struct SilentAssertReporter {
    buffer: Mutex<Vec<String>>,
}

impl SilentAssertReporter {
    /// Create a new assertion reporter with an empty output buffer.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buffer: Mutex::new(Vec::new()),
        }
    }

    /// Snapshot the buffered step/output lines captured so far, e.g. for a
    /// caller that wants to assert on captured context directly rather than
    /// relying on `failure()`'s panic message.
    #[must_use]
    pub fn captured(&self) -> Vec<String> {
        self.buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Default for SilentAssertReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl Reporter for SilentAssertReporter {
    fn section(&self, _name: &str) {}

    fn step(&self, name: &str) {
        self.buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("step: {name}"));
    }

    fn output(&self, line: &str) {
        self.buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(line.to_string());
    }

    fn success(&self, _msg: &str) {}

    fn skip(&self, reason: &str) {
        // Caller (test fn) is expected to check its own early-return flag,
        // e.g. gating on `BackendCapabilitySet::supports()` before running
        // assertions, matching `ConformanceTest::required_capability()`
        // gating. skip() just records the reason and surfaces it on
        // stdout so `cargo nextest` output shows why a scenario stopped
        // short, without failing the test.
        let ctx_line = format!("skip: {reason}");
        self.buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(ctx_line);
        println!("SKIP: {reason}");
    }

    #[allow(clippy::assertions_on_constants)]
    fn failure(&self, msg: &str) {
        let ctx = self
            .buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .join("\n");
        // Panic-style helpers and direct unwrap/expect calls are denied by workspace lints;
        // `assert!` is this codebase's established test-harness idiom for
        // an unconditional failure (see `context::fail`).
        assert!(false, "{msg}\n--- captured output ---\n{ctx}");
    }
}

impl fmt::Debug for SilentAssertReporter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SilentAssertReporter")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_reporter_buffers_steps_and_output() {
        let reporter = SilentAssertReporter::new();
        reporter.step("pulling alpine");
        reporter.output("layer sha256:abcd downloaded");
        let captured = reporter.captured();
        assert_eq!(captured.len(), 2);
        assert!(captured[0].contains("pulling alpine"));
        assert!(captured[1].contains("sha256:abcd"));
    }

    #[test]
    #[should_panic(expected = "container failed to start")]
    fn silent_reporter_failure_panics_with_context() {
        let reporter = SilentAssertReporter::new();
        reporter.step("starting container");
        reporter.failure("container failed to start");
    }

    #[test]
    fn silent_reporter_skip_records_reason_without_panicking() {
        let reporter = SilentAssertReporter::new();
        reporter.skip("backend does not support BuildFromContext");
        assert!(
            reporter
                .captured()
                .iter()
                .any(|line| line.contains("BuildFromContext"))
        );
    }

    #[test]
    fn narrated_reporter_methods_do_not_panic() {
        let reporter = NarratedReporter::new();
        reporter.section("demo scenario");
        reporter.step("doing a thing");
        reporter.output("some subprocess line");
        reporter.success("thing done");
        reporter.skip("not applicable on this platform");
        reporter.failure("simulated failure for narrated mode");
        reporter.summary();
    }

    #[test]
    fn reporter_trait_object_is_usable_dynamically() {
        let narrated: Box<dyn Reporter> = Box::new(NarratedReporter::new());
        let silent: Box<dyn Reporter> = Box::new(SilentAssertReporter::new());
        for reporter in [narrated, silent] {
            reporter.section("dyn dispatch check");
            reporter.step("step");
            reporter.success("ok");
        }
    }
}
