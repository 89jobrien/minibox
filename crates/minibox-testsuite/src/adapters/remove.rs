//! Conformance tests for the `handle_remove` handler contract.
//!
//! Tests exercise `handle_remove` via mock `HandlerDependencies` so no
//! real filesystem or cgroup operations are performed.

use minibox::daemon::handler::handle_remove;
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

/// `handle_remove` with an unknown container ID must return `DaemonResponse::Error`.
pub struct RemoveUnknownContainerReturnsError;

impl ConformanceTest for RemoveUnknownContainerReturnsError {
    fn name(&self) -> &str {
        "remove_unknown_container_returns_error"
    }
    fn adapter(&self) -> &str {
        "remove"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);

        let resp = rt().block_on(handle_remove("doesnotexist0001".to_string(), state, deps));

        ctx.assert_true(
            matches!(resp, DaemonResponse::Error { .. }),
            "remove of unknown container returns Error",
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

/// `handle_remove` on a stopped container returns `Success` and removes it from state.
pub struct RemoveStoppedContainerSucceeds;

impl ConformanceTest for RemoveStoppedContainerSucceeds {
    fn name(&self) -> &str {
        "remove_stopped_container_succeeds"
    }
    fn adapter(&self) -> &str {
        "remove"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);

        let id = "removestop0000001".to_string();
        let mut record = make_stub_record(id.clone());
        record.info.state = "Stopped".to_string();

        rt().block_on(state.add_container(record));

        let resp = rt().block_on(handle_remove(id.clone(), state.clone(), deps));

        ctx.assert_true(
            matches!(resp, DaemonResponse::Success { .. }),
            "remove of stopped container returns Success",
        );

        // Container must no longer be present in state.
        let still_present = rt().block_on(state.get_container(&id));
        ctx.assert_true(still_present.is_none(), "container removed from state");

        ctx.result()
    }
}

/// `handle_remove` on a running container must return `DaemonResponse::Error`.
pub struct RemoveRunningContainerReturnsError;

impl ConformanceTest for RemoveRunningContainerReturnsError {
    fn name(&self) -> &str {
        "remove_running_container_returns_error"
    }
    fn adapter(&self) -> &str {
        "remove"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);

        let id = "removerunning0001".to_string();
        let mut record = make_stub_record(id.clone());
        record.info.state = "Running".to_string();
        record.pid = Some(99999); // fictional PID — handler checks state, not PID

        rt().block_on(state.add_container(record));

        let resp = rt().block_on(handle_remove(id.clone(), state.clone(), deps));

        ctx.assert_true(
            matches!(resp, DaemonResponse::Error { .. }),
            "remove of running container returns Error",
        );

        // Container must still be present (not removed on error).
        let still_present = rt().block_on(state.get_container(&id));
        ctx.assert_true(
            still_present.is_some(),
            "running container not removed from state on error",
        );

        ctx.result()
    }
}

/// After remove, `list_containers` no longer includes the removed container.
pub struct RemoveReducesListCount;

impl ConformanceTest for RemoveReducesListCount {
    fn name(&self) -> &str {
        "remove_reduces_list_count"
    }
    fn adapter(&self) -> &str {
        "remove"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Integration
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);

        // Add two stopped containers.
        for id in &["removelist000001", "removelist000002"] {
            let mut record = make_stub_record(*id);
            record.info.state = "Stopped".to_string();
            rt().block_on(state.add_container(record));
        }

        ctx.assert_eq(
            2,
            rt().block_on(state.list_containers()).len(),
            "two containers before remove",
        );

        let resp = rt().block_on(handle_remove(
            "removelist000001".to_string(),
            state.clone(),
            deps,
        ));
        ctx.assert_true(
            matches!(resp, DaemonResponse::Success { .. }),
            "remove returns Success",
        );

        ctx.assert_eq(
            1,
            rt().block_on(state.list_containers()).len(),
            "one container after remove",
        );

        ctx.result()
    }
}

/// `handle_remove` resolves containers by name as well as by ID.
pub struct RemoveByNameSucceeds;

impl ConformanceTest for RemoveByNameSucceeds {
    fn name(&self) -> &str {
        "remove_by_name_succeeds"
    }
    fn adapter(&self) -> &str {
        "remove"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);

        let id = "removebyname00001".to_string();
        let mut record = make_stub_record(id.clone());
        record.info.name = Some("mycontainer".to_string());
        record.info.state = "Stopped".to_string();

        rt().block_on(state.add_container(record));

        // Remove by name, not by ID.
        let resp = rt().block_on(handle_remove(
            "mycontainer".to_string(),
            state.clone(),
            deps,
        ));

        ctx.assert_true(
            matches!(resp, DaemonResponse::Success { .. }),
            "remove by name returns Success",
        );

        let still_present = rt().block_on(state.get_container(&id));
        ctx.assert_true(
            still_present.is_none(),
            "container removed from state by name",
        );

        ctx.result()
    }
}

/// Return all remove conformance tests.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(RemoveUnknownContainerReturnsError),
        Box::new(RemoveStoppedContainerSucceeds),
        Box::new(RemoveRunningContainerReturnsError),
        Box::new(RemoveReducesListCount),
        Box::new(RemoveByNameSucceeds),
    ]
}
