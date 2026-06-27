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
    fn name(&self) -> &'static str {
        "logs_unknown_container_returns_error"
    }
    fn adapter(&self) -> &'static str {
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
    fn name(&self) -> &'static str {
        "logs_stopped_container_returns_empty_or_error"
    }
    fn adapter(&self) -> &'static str {
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
    fn name(&self) -> &'static str {
        "logs_follow_false_terminates"
    }
    fn adapter(&self) -> &'static str {
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

/// `handle_logs` for a container registered with state="running" must not
/// return an `Error` — it must terminate with `Success` after draining available
/// log output (which may be empty when no log files are present).
pub struct LogsRunningContainerTerminatesWithSuccess;

impl ConformanceTest for LogsRunningContainerTerminatesWithSuccess {
    fn name(&self) -> &'static str {
        "logs_running_container_terminates_with_success"
    }
    fn adapter(&self) -> &'static str {
        "logs"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);

        // Register a container whose state field is "running".
        let mut record = make_stub_record("running-ctr-aabbccdd1122");
        record.info.state = "running".to_string();
        rt().block_on(state.add_container(record));

        let (tx, mut rx) = mpsc::channel::<DaemonResponse>(32);
        rt().block_on(async {
            handle_logs(
                "running-ctr-aabbccdd1122".to_string(),
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
        ctx.assert_false(
            got_error,
            "handle_logs for running container must not return Error",
        );
        let got_success = responses
            .iter()
            .any(|r| matches!(r, DaemonResponse::Success { .. }));
        ctx.assert_true(
            got_success,
            "handle_logs for running container must terminate with Success",
        );
        ctx.result()
    }
}

/// When a container exists but has no log files on disk, `handle_logs` must
/// emit zero `LogLine` responses and then terminate with `Success`.
pub struct LogsEmptyOutputHasNoLogLines;

impl ConformanceTest for LogsEmptyOutputHasNoLogLines {
    fn name(&self) -> &'static str {
        "logs_empty_output_has_no_log_lines"
    }
    fn adapter(&self) -> &'static str {
        "logs"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);

        let record = make_stub_record("empty-log-ctr-aabbccdd1122");
        rt().block_on(state.add_container(record));

        let (tx, mut rx) = mpsc::channel::<DaemonResponse>(32);
        rt().block_on(async {
            handle_logs(
                "empty-log-ctr-aabbccdd1122".to_string(),
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

        let log_line_count = responses
            .iter()
            .filter(|r| matches!(r, DaemonResponse::LogLine { .. }))
            .count();
        ctx.assert_eq(
            0,
            log_line_count,
            "zero LogLine responses when no log files exist",
        );

        let got_success = responses
            .iter()
            .any(|r| matches!(r, DaemonResponse::Success { .. }));
        ctx.assert_true(
            got_success,
            "handle_logs terminates with Success when log files are absent",
        );
        ctx.result()
    }
}

/// `handle_logs` with a stdout.log file containing known lines must emit one
/// `LogLine` per line, in order, followed by `Success`.
pub struct LogsWithStdoutFileEmitsLogLines;

impl ConformanceTest for LogsWithStdoutFileEmitsLogLines {
    fn name(&self) -> &'static str {
        "logs_with_stdout_file_emits_log_lines"
    }
    fn adapter(&self) -> &'static str {
        "logs"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);

        let ctr_id = "log-lines-ctr-aabbccdd1122";
        let record = make_stub_record(ctr_id);
        rt().block_on(state.add_container(record));

        // Write a stdout.log file inside the container's log directory.
        let log_dir = tmp.path().join("containers").join(ctr_id);
        std::fs::create_dir_all(&log_dir).expect("create log dir");
        std::fs::write(
            log_dir.join("stdout.log"),
            "line one\nline two\nline three\n",
        )
        .expect("write stdout.log");

        let (tx, mut rx) = mpsc::channel::<DaemonResponse>(64);
        rt().block_on(async {
            handle_logs(ctr_id.to_string(), false, state, deps, tx).await;
        });

        let responses: Vec<DaemonResponse> = rt().block_on(async {
            let mut out = Vec::new();
            while let Ok(msg) = rx.try_recv() {
                out.push(msg);
            }
            out
        });

        let log_lines: Vec<&str> = responses
            .iter()
            .filter_map(|r| {
                if let DaemonResponse::LogLine { line, .. } = r {
                    Some(line.as_str())
                } else {
                    None
                }
            })
            .collect();

        ctx.assert_eq(
            3,
            log_lines.len(),
            "three LogLine responses for three-line stdout.log",
        );
        if log_lines.len() == 3 {
            ctx.assert_eq("line one", log_lines[0], "first LogLine content");
            ctx.assert_eq("line two", log_lines[1], "second LogLine content");
            ctx.assert_eq("line three", log_lines[2], "third LogLine content");
        }

        let got_success = responses
            .iter()
            .any(|r| matches!(r, DaemonResponse::Success { .. }));
        ctx.assert_true(
            got_success,
            "handle_logs terminates with Success after LogLines",
        );
        ctx.result()
    }
}

/// Return all logs conformance tests.
#[must_use]
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(LogsUnknownContainerReturnsError),
        Box::new(LogsStoppedContainerReturnsEmptyOrError),
        Box::new(LogsFollowFalseTerminates),
        Box::new(LogsRunningContainerTerminatesWithSuccess),
        Box::new(LogsEmptyOutputHasNoLogLines),
        Box::new(LogsWithStdoutFileEmitsLogLines),
    ]
}
