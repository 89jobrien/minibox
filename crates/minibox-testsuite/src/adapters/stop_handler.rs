//! Conformance tests for the `handle_stop` handler contract.
//!
//! Only error paths and state-visible side-effects are tested here — the
//! success path requires sending a real UNIX signal to a live process,
//! which is out of scope for backend-agnostic conformance tests.

use minibox::daemon::handler::handle_stop;
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
    name: "stop_unknown_container_returns_error",
    adapter: "stop_handler",
    category: EdgeCase,
    |ctx| {
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

crate::conformance_test! {
    name: "stop_no_pid_container_returns_error",
    adapter: "stop_handler",
    category: EdgeCase,
    |ctx| {
        let tmp = TempDir::new().expect("TempDir::new");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);

        let id = "stopnopid00000001".to_string();
        let mut record = make_stub_record(id.clone());
        record.pid = None;
        record.info.state = "Created".to_string();

        rt().block_on(state.add_container(record));

        let resp = rt().block_on(handle_stop(id, state, deps));

        // Container has no PID — stop_inner returns an error on Unix.
        ctx.assert_true(
            matches!(resp, DaemonResponse::Error { .. }),
            "stop of no-PID container returns Error",
        );

        ctx.result()
    }
}

crate::conformance_test! {
    name: "stop_unknown_name_returns_error",
    adapter: "stop_handler",
    category: EdgeCase,
    |ctx| {
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
