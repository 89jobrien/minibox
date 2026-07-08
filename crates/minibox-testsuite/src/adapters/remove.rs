//! Conformance tests for the `handle_remove` handler contract.
//!
//! Tests exercise `handle_remove` via mock `HandlerDependencies` so no
//! real filesystem or cgroup operations are performed.

use minibox::daemon::handler::handle_remove;
use minibox::testing::helpers::daemon::{make_mock_deps, make_mock_state, make_stub_record};
use minibox_core::protocol::DaemonResponse;
use tempfile::TempDir;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build Tokio runtime")
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

crate::conformance_test! {
    name: "remove_unknown_container_returns_error",
    adapter: "remove",
    category: EdgeCase,
    |ctx| {
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

crate::conformance_test! {
    name: "remove_stopped_container_succeeds",
    adapter: "remove",
    category: Unit,
    |ctx| {
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

crate::conformance_test! {
    name: "remove_running_container_returns_error",
    adapter: "remove",
    category: EdgeCase,
    |ctx| {
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

crate::conformance_test! {
    name: "remove_reduces_list_count",
    adapter: "remove",
    category: Integration,
    |ctx| {
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

crate::conformance_test! {
    name: "remove_by_name_succeeds",
    adapter: "remove",
    category: Unit,
    |ctx| {
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
