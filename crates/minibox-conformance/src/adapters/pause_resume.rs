//! Conformance tests for pause/resume handler contracts.
//!
//! These tests exercise `handle_pause` and `handle_resume` against a real
//! `DaemonState`. The cgroup write succeeds when a writable `cgroup.freeze`
//! file is pre-created in a temp directory; error-path tests use a
//! non-existent path so the write fails naturally.

use minibox::daemon::handler;
use minibox::daemon::state::{ContainerRecord, DaemonState};
use minibox_core::domain::ContainerState;
use minibox_core::events::NoopEventSink;
use minibox_core::image::ImageStore;
use minibox_core::protocol::{ContainerInfo, DaemonResponse};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

fn make_state(tmp: &TempDir) -> Arc<DaemonState> {
    let image_store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");
    Arc::new(DaemonState::new(image_store, tmp.path()))
}

/// Build a minimal `ContainerRecord` with the given state string and cgroup path.
fn make_record(id: &str, state_str: &str, cgroup_path: PathBuf) -> ContainerRecord {
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: state_str.to_string(),
            created_at: "2026-04-27T00:00:00Z".to_string(),
            pid: Some(1234),
        },
        pid: Some(1234),
        rootfs_path: PathBuf::from("/mock/rootfs"),
        cgroup_path,
        post_exit_hooks: vec![],
        rootfs_metadata: None,
        source_image_ref: None,
        step_state: None,
        priority: None,
        urgency: None,
        execution_context: None,
        creation_params: None,
        manifest_path: None,
        workload_digest: None,
    }
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

/// Pausing a Running container writes `cgroup.freeze` and transitions to Paused.
pub struct PauseRunningContainerTransitionsToPaused;
impl ConformanceTest for PauseRunningContainerTransitionsToPaused {
    fn name(&self) -> &str {
        "pause_running_container_transitions_to_paused"
    }
    fn adapter(&self) -> &str {
        "pause_resume"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Integration
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        // Pre-create cgroup.freeze so the handler write succeeds.
        let cgroup_dir = tmp.path().join("cgroup");
        std::fs::create_dir_all(&cgroup_dir).expect("create cgroup dir");
        std::fs::write(cgroup_dir.join("cgroup.freeze"), "0\n").expect("write cgroup.freeze");

        let state = make_state(&tmp);
        let id = "pauserunning0001".to_string();
        rt().block_on(
            state.add_container(make_record(&id, "Running", cgroup_dir.clone())),
        );

        let resp = rt().block_on(handler::handle_pause(
            id.clone(),
            state.clone(),
            Arc::new(NoopEventSink),
        ));

        ctx.assert_true(
            matches!(resp, DaemonResponse::ContainerPaused { .. }),
            "pause of Running container returns ContainerPaused",
        );

        let record = rt().block_on(state.get_container(&id));
        if let Some(r) = record {
            ctx.assert_eq(
                ContainerState::Paused.as_str().to_string(),
                r.info.state,
                "state transitions to Paused",
            );
        } else {
            ctx.assert_true(false, "container record should still exist after pause");
        }

        ctx.result()
    }
}

/// Resuming a Paused container writes `cgroup.freeze` and transitions to Running.
pub struct ResumePausedContainerTransitionsToRunning;
impl ConformanceTest for ResumePausedContainerTransitionsToRunning {
    fn name(&self) -> &str {
        "resume_paused_container_transitions_to_running"
    }
    fn adapter(&self) -> &str {
        "pause_resume"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Integration
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let cgroup_dir = tmp.path().join("cgroup");
        std::fs::create_dir_all(&cgroup_dir).expect("create cgroup dir");
        std::fs::write(cgroup_dir.join("cgroup.freeze"), "1\n").expect("write cgroup.freeze");

        let state = make_state(&tmp);
        let id = "resumepaused0001".to_string();
        rt().block_on(
            state.add_container(make_record(&id, "Paused", cgroup_dir.clone())),
        );

        let resp = rt().block_on(handler::handle_resume(
            id.clone(),
            state.clone(),
            Arc::new(NoopEventSink),
        ));

        ctx.assert_true(
            matches!(resp, DaemonResponse::ContainerResumed { .. }),
            "resume of Paused container returns ContainerResumed",
        );

        let record = rt().block_on(state.get_container(&id));
        if let Some(r) = record {
            ctx.assert_eq(
                ContainerState::Running.as_str().to_string(),
                r.info.state,
                "state transitions to Running",
            );
        } else {
            ctx.assert_true(false, "container record should still exist after resume");
        }

        ctx.result()
    }
}

/// Pausing an already-Paused container returns an error.
pub struct PauseAlreadyPausedReturnsError;
impl ConformanceTest for PauseAlreadyPausedReturnsError {
    fn name(&self) -> &str {
        "pause_already_paused_returns_error"
    }
    fn adapter(&self) -> &str {
        "pause_resume"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_state(&tmp);
        let id = "pausepaused00001".to_string();
        // cgroup_path doesn't matter — handler checks state before writing.
        rt().block_on(state.add_container(make_record(
            &id,
            "Paused",
            PathBuf::from("/nonexistent/cgroup"),
        )));

        let resp = rt().block_on(handler::handle_pause(
            id.clone(),
            state.clone(),
            Arc::new(NoopEventSink),
        ));

        ctx.assert_true(
            matches!(resp, DaemonResponse::Error { .. }),
            "pause of already-Paused container returns Error",
        );
        if let DaemonResponse::Error { message } = resp {
            ctx.assert_true(
                message.contains("not running"),
                "error message mentions 'not running'",
            );
        }

        ctx.result()
    }
}

/// Resuming a Running (non-paused) container returns an error.
pub struct ResumeRunningContainerReturnsNotPausedError;
impl ConformanceTest for ResumeRunningContainerReturnsNotPausedError {
    fn name(&self) -> &str {
        "resume_running_container_returns_not_paused_error"
    }
    fn adapter(&self) -> &str {
        "pause_resume"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_state(&tmp);
        let id = "resumerunning001".to_string();
        rt().block_on(state.add_container(make_record(
            &id,
            "Running",
            PathBuf::from("/nonexistent/cgroup"),
        )));

        let resp = rt().block_on(handler::handle_resume(
            id.clone(),
            state.clone(),
            Arc::new(NoopEventSink),
        ));

        ctx.assert_true(
            matches!(resp, DaemonResponse::Error { .. }),
            "resume of Running container returns Error",
        );
        if let DaemonResponse::Error { message } = resp {
            ctx.assert_true(
                message.contains("not paused"),
                "error message mentions 'not paused'",
            );
        }

        ctx.result()
    }
}

/// Pausing an unknown container returns an error.
pub struct PauseUnknownContainerReturnsError;
impl ConformanceTest for PauseUnknownContainerReturnsError {
    fn name(&self) -> &str {
        "pause_unknown_container_returns_error"
    }
    fn adapter(&self) -> &str {
        "pause_resume"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_state(&tmp);

        let resp = rt().block_on(handler::handle_pause(
            "doesnotexist0001".to_string(),
            state.clone(),
            Arc::new(NoopEventSink),
        ));

        ctx.assert_true(
            matches!(resp, DaemonResponse::Error { .. }),
            "pause of unknown container returns Error",
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

/// Return all pause/resume conformance tests.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(PauseRunningContainerTransitionsToPaused),
        Box::new(ResumePausedContainerTransitionsToRunning),
        Box::new(PauseAlreadyPausedReturnsError),
        Box::new(ResumeRunningContainerReturnsNotPausedError),
        Box::new(PauseUnknownContainerReturnsError),
    ]
}
