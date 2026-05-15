//! Conformance tests for container-ID validation edge cases.
//!
//! All tests verify that daemon handlers correctly reject unknown, empty, or
//! mismatched container IDs without touching the filesystem or real processes.
//! Mock adapters are used throughout; no syscalls are made.

use std::sync::Arc;

use minibox::daemon::handler::{handle_pause, handle_remove, handle_resume, handle_stop};
use minibox::testing::helpers::daemon::{make_mock_deps, make_mock_state, make_stub_record};
use minibox_core::events::NoopEventSink;
use minibox_core::protocol::DaemonResponse;
use tempfile::TempDir;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build Tokio runtime")
}

fn is_error(resp: &DaemonResponse) -> bool {
    matches!(resp, DaemonResponse::Error { .. })
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

pub struct StopEmptyIdReturnsError;
impl ConformanceTest for StopEmptyIdReturnsError {
    fn name(&self) -> &str {
        "stop_empty_id_returns_error"
    }
    fn adapter(&self) -> &str {
        "container_id"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("tempdir");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);
        let resp = rt().block_on(handle_stop("".to_string(), state, deps));
        ctx.assert_true(is_error(&resp), "stop with empty id returns Error response");
        ctx.result()
    }
}

pub struct RemoveUnknownIdReturnsError;
impl ConformanceTest for RemoveUnknownIdReturnsError {
    fn name(&self) -> &str {
        "remove_unknown_id_returns_error"
    }
    fn adapter(&self) -> &str {
        "container_id"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("tempdir");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);
        let resp =
            rt().block_on(handle_remove("definitely-not-real".to_string(), state, deps));
        ctx.assert_true(is_error(&resp), "remove with unknown id returns Error response");
        ctx.result()
    }
}

pub struct PauseUnknownIdReturnsError;
impl ConformanceTest for PauseUnknownIdReturnsError {
    fn name(&self) -> &str {
        "pause_unknown_id_returns_error"
    }
    fn adapter(&self) -> &str {
        "container_id"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("tempdir");
        let state = make_mock_state(tmp.path());
        let event_sink = Arc::new(NoopEventSink) as Arc<dyn minibox_core::events::EventSink>;
        let resp =
            rt().block_on(handle_pause("ghost-container".to_string(), state, event_sink));
        ctx.assert_true(is_error(&resp), "pause with unknown id returns Error response");
        ctx.result()
    }
}

pub struct ResumeUnknownIdReturnsError;
impl ConformanceTest for ResumeUnknownIdReturnsError {
    fn name(&self) -> &str {
        "resume_unknown_id_returns_error"
    }
    fn adapter(&self) -> &str {
        "container_id"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("tempdir");
        let state = make_mock_state(tmp.path());
        let event_sink = Arc::new(NoopEventSink) as Arc<dyn minibox_core::events::EventSink>;
        let resp =
            rt().block_on(handle_resume("ghost-container".to_string(), state, event_sink));
        ctx.assert_true(is_error(&resp), "resume with unknown id returns Error response");
        ctx.result()
    }
}

pub struct IdsAreCaseSensitive;
impl ConformanceTest for IdsAreCaseSensitive {
    fn name(&self) -> &str {
        "ids_are_case_sensitive"
    }
    fn adapter(&self) -> &str {
        "container_id"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("tempdir");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);

        // Add a container with a mixed-case ID.
        let record = make_stub_record("MyContainer");
        rt().block_on(state.add_container(record));

        // Attempting to stop with the lowercase variant must fail — IDs are case-sensitive.
        let resp =
            rt().block_on(handle_stop("mycontainer".to_string(), Arc::clone(&state), deps));
        ctx.assert_true(
            is_error(&resp),
            "stop with wrong-case id returns Error (ids are case-sensitive)",
        );
        ctx.result()
    }
}

/// Return all container-ID edge case conformance tests.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(StopEmptyIdReturnsError),
        Box::new(RemoveUnknownIdReturnsError),
        Box::new(PauseUnknownIdReturnsError),
        Box::new(ResumeUnknownIdReturnsError),
        Box::new(IdsAreCaseSensitive),
    ]
}
