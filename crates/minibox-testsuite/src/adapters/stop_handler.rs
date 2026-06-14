//! Conformance tests for the `handle_stop` handler contract.
//!
//! Only error paths and state-visible side-effects are tested here — the
//! success path requires sending a real UNIX signal to a live process,
//! which is out of scope for backend-agnostic conformance tests.

use minibox::daemon::handler::handle_stop;
use minibox::testing::helpers::daemon::{make_mock_deps, make_mock_state, make_stub_record};
use minibox_core::protocol::DaemonResponse;
use tempfile::TempDir;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build Tokio runtime")
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

/// `handle_stop` with an unknown container ID returns `DaemonResponse::Error`.
pub struct StopUnknownContainerReturnsError;

impl ConformanceTest for StopUnknownContainerReturnsError {
    fn name(&self) -> &str {
        "stop_unknown_container_returns_error"
    }
    fn adapter(&self) -> &str {
        "stop_handler"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);

        let resp = rt().block_on(handle_stop("doesnotexist0001".to_string(), state, deps));

        ctx.assert_true(
            matches!(resp, DaemonResponse::Error { .. }),
            "stop of unknown container returns Error",
        );
        if let DaemonResponse::Error { message } = resp {
            ctx.assert_true(
                message.contains("not found"),
                "error message mentions 'not found'",
            );
        }
        ctx.result()
    }
}

/// `handle_stop` with a container that has no PID returns `DaemonResponse::Error`.
///
/// A container in `Created` state has no PID set. Attempting to stop it should
/// fail cleanly rather than panic.
pub struct StopNoPidContainerReturnsError;

impl ConformanceTest for StopNoPidContainerReturnsError {
    fn name(&self) -> &str {
        "stop_no_pid_container_returns_error"
    }
    fn adapter(&self) -> &str {
        "stop_handler"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);

        let id = "stopnopid00000001".to_string();
        let mut record = make_stub_record(id.clone());
        record.pid = None;
        record.info.state = "Created".to_string();

        rt().block_on(state.add_container(record));

        let resp = rt().block_on(handle_stop(id.clone(), state.clone(), deps));

        // Container has no PID — stop_inner returns an error on Unix.
        ctx.assert_true(
            matches!(resp, DaemonResponse::Error { .. }),
            "stop of no-PID container returns Error",
        );

        ctx.result()
    }
}

/// `handle_stop` resolves containers by name as well as by ID.
pub struct StopUnknownNameReturnsError;

impl ConformanceTest for StopUnknownNameReturnsError {
    fn name(&self) -> &str {
        "stop_unknown_name_returns_error"
    }
    fn adapter(&self) -> &str {
        "stop_handler"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);

        // Stop by a name that has never been registered.
        let resp = rt().block_on(handle_stop("nonexistent-name".to_string(), state, deps));

        ctx.assert_true(
            matches!(resp, DaemonResponse::Error { .. }),
            "stop by unknown name returns Error",
        );
        ctx.result()
    }
}

/// Return all stop handler conformance tests.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(StopUnknownContainerReturnsError),
        Box::new(StopNoPidContainerReturnsError),
        Box::new(StopUnknownNameReturnsError),
    ]
}
