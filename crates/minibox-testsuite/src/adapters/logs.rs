//! Conformance tests for the `handle_logs` handler contract.

use minibox::daemon::handler::handle_logs;
use minibox::testing::helpers::daemon::{make_mock_deps, make_mock_state, make_stub_record};
use minibox_core::protocol::DaemonResponse;
use tempfile::TempDir;
use tokio::sync::mpsc;

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

/// `handle_logs` for a container ID that does not exist must return
/// `DaemonResponse::Error`.
pub struct LogsUnknownContainerReturnsError;

impl ConformanceTest for LogsUnknownContainerReturnsError {
    fn name(&self) -> &str {
        "logs_unknown_container_returns_error"
    }
    fn adapter(&self) -> &str {
        "logs"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);
        let (tx, mut rx) = mpsc::channel::<DaemonResponse>(16);

        rt().block_on(async {
            handle_logs(
                "nonexistent-container-id".to_string(),
                false,
                state,
                deps,
                tx,
            )
            .await;
        });

        let responses: Vec<DaemonResponse> = rt().block_on(async {
            let mut out = Vec::new();
            while let Ok(msg) = rx.try_recv() {
                out.push(msg);
            }
            out
        });

        let got_error = responses
            .iter()
            .any(|r| matches!(r, DaemonResponse::Error { .. }));
        ctx.assert_true(got_error, "handle_logs for unknown container returns Error");
        ctx.result()
    }
}

/// `handle_logs` for a stopped container with no log files must return either
/// zero `LogLine` responses followed by `Success`, or an `Error` — both are
/// conformant.  It must NOT block indefinitely.
pub struct LogsStoppedContainerReturnsEmptyOrError;

impl ConformanceTest for LogsStoppedContainerReturnsEmptyOrError {
    fn name(&self) -> &str {
        "logs_stopped_container_returns_empty_or_error"
    }
    fn adapter(&self) -> &str {
        "logs"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);

        // Register a stopped container in state (no log files on disk).
        let record = make_stub_record("stopped-ctr-aabbccdd1122");
        rt().block_on(state.add_container(record));

        let (tx, mut rx) = mpsc::channel::<DaemonResponse>(16);
        rt().block_on(async {
            handle_logs(
                "stopped-ctr-aabbccdd1122".to_string(),
                false,
                state,
                deps,
                tx,
            )
            .await;
        });

        let responses: Vec<DaemonResponse> = rt().block_on(async {
            let mut out = Vec::new();
            while let Ok(msg) = rx.try_recv() {
                out.push(msg);
            }
            out
        });

        // Either an Error or a Success terminal response is conformant.
        let terminal_ok = responses.iter().any(|r| {
            matches!(
                r,
                DaemonResponse::Error { .. } | DaemonResponse::Success { .. }
            )
        });
        ctx.assert_true(
            terminal_ok,
            "handle_logs for stopped container with no logs terminates with Error or Success",
        );
        ctx.result()
    }
}

/// With `follow = false`, `handle_logs` must close the channel (terminate the
/// stream) after sending any available output.  The tx is dropped by the
/// handler, causing the rx to return `None` from `recv()`.
pub struct LogsFollowFalseTerminates;

impl ConformanceTest for LogsFollowFalseTerminates {
    fn name(&self) -> &str {
        "logs_follow_false_terminates"
    }
    fn adapter(&self) -> &str {
        "logs"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);

        // Register a container so the lookup succeeds.
        let record = make_stub_record("follow-false-aabbccdd1122");
        rt().block_on(state.add_container(record));

        let (tx, mut rx) = mpsc::channel::<DaemonResponse>(64);

        rt().block_on(async {
            handle_logs(
                "follow-false-aabbccdd1122".to_string(),
                false,
                state,
                deps,
                tx,
            )
            .await;
        });
        // After handle_logs returns (and drops tx), the receiver must be
        // exhausted — recv() must return None, not block forever.
        let stream_closed = rt().block_on(async { rx.recv().await }).is_none()
            || rt().block_on(async {
                // drain remaining then check closed
                while rx.try_recv().is_ok() {}
                rx.recv().await.is_none()
            });

        ctx.assert_true(stream_closed, "logs channel closes after follow=false");
        ctx.result()
    }
}

/// Return all logs conformance tests.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(LogsUnknownContainerReturnsError),
        Box::new(LogsStoppedContainerReturnsEmptyOrError),
        Box::new(LogsFollowFalseTerminates),
    ]
}
