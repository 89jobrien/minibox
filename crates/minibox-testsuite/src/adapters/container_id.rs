//! Conformance tests for container-ID validation edge cases.
//!
//! Two groups of tests:
//!
//! 1. `ContainerId::new` — unit-level validation of the domain type.
//! 2. Handler-level — daemon handlers reject unknown/empty/mismatched IDs
//!    without touching the filesystem or real processes.

use std::sync::Arc;

use minibox::daemon::handler::{handle_pause, handle_remove, handle_resume, handle_stop};
use minibox::testing::helpers::daemon::{make_mock_deps, make_mock_state, make_stub_record};
use minibox_core::domain::ContainerId;
use minibox_core::events::NoopEventSink;
use minibox_core::protocol::DaemonResponse;
use tempfile::TempDir;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build Tokio runtime")
}

const fn is_error(resp: &DaemonResponse) -> bool {
    matches!(resp, DaemonResponse::Error { .. })
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

crate::conformance_test! {
    name: "stop_empty_id_returns_error",
    adapter: "container_id",
    category: EdgeCase,
    |ctx| {
        let tmp = TempDir::new().expect("tempdir");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);
        let resp = rt().block_on(handle_stop(String::new(), state, deps));
        ctx.assert_true(is_error(&resp), "stop with empty id returns Error response");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "remove_unknown_id_returns_error",
    adapter: "container_id",
    category: EdgeCase,
    |ctx| {
        let tmp = TempDir::new().expect("tempdir");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);
        let resp = rt().block_on(handle_remove(
            "definitely-not-real".to_string(),
            state,
            deps,
        ));
        ctx.assert_true(
            is_error(&resp),
            "remove with unknown id returns Error response",
        );
        ctx.result()
    }
}

crate::conformance_test! {
    name: "pause_unknown_id_returns_error",
    adapter: "container_id",
    category: EdgeCase,
    |ctx| {
        let tmp = TempDir::new().expect("tempdir");
        let state = make_mock_state(tmp.path());
        let event_sink = Arc::new(NoopEventSink) as Arc<dyn minibox_core::events::EventSink>;
        let resp = rt().block_on(handle_pause(
            "ghost-container".to_string(),
            state,
            event_sink,
        ));
        ctx.assert_true(
            is_error(&resp),
            "pause with unknown id returns Error response",
        );
        ctx.result()
    }
}

crate::conformance_test! {
    name: "resume_unknown_id_returns_error",
    adapter: "container_id",
    category: EdgeCase,
    |ctx| {
        let tmp = TempDir::new().expect("tempdir");
        let state = make_mock_state(tmp.path());
        let event_sink = Arc::new(NoopEventSink) as Arc<dyn minibox_core::events::EventSink>;
        let resp = rt().block_on(handle_resume(
            "ghost-container".to_string(),
            state,
            event_sink,
        ));
        ctx.assert_true(
            is_error(&resp),
            "resume with unknown id returns Error response",
        );
        ctx.result()
    }
}

crate::conformance_test! {
    name: "ids_are_case_sensitive",
    adapter: "container_id",
    category: EdgeCase,
    |ctx| {
        let tmp = TempDir::new().expect("tempdir");
        let state = make_mock_state(tmp.path());
        let deps = make_mock_deps(&tmp);

        // Add a container with a mixed-case ID.
        let record = make_stub_record("MyContainer");
        rt().block_on(state.add_container(record));

        // Attempting to stop with the lowercase variant must fail — IDs are case-sensitive.
        let resp = rt().block_on(handle_stop(
            "mycontainer".to_string(),
            Arc::clone(&state),
            deps,
        ));
        ctx.assert_true(
            is_error(&resp),
            "stop with wrong-case id returns Error (ids are case-sensitive)",
        );
        ctx.result()
    }
}

// ---------------------------------------------------------------------------
// ContainerId::new — domain-type validation tests
// ---------------------------------------------------------------------------

crate::conformance_test! {
    name: "container_id_empty_string_rejected",
    adapter: "container_id",
    category: EdgeCase,
    |ctx| {
        let result = ContainerId::new(String::new());
        ctx.assert_err(result, "ContainerId::new rejects empty string");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "container_id_whitespace_only_rejected",
    adapter: "container_id",
    category: EdgeCase,
    |ctx| {
        let result = ContainerId::new("   ".to_string());
        ctx.assert_err(result, "ContainerId::new rejects whitespace-only string");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "container_id_too_long_rejected",
    adapter: "container_id",
    category: EdgeCase,
    |ctx| {
        // 65 alphanumeric characters — one over the 64-char limit.
        let long_id = "a".repeat(65);
        let result = ContainerId::new(long_id);
        ctx.assert_err(result, "ContainerId::new rejects IDs longer than 64 chars");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "container_id_at_max_length_accepted",
    adapter: "container_id",
    category: Unit,
    |ctx| {
        // Exactly 64 alphanumeric characters — at the limit.
        let max_id = "a".repeat(64);
        let result = ContainerId::new(max_id);
        ctx.assert_ok(result, "ContainerId::new accepts 64-char alphanumeric ID");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "container_id_special_chars_rejected",
    adapter: "container_id",
    category: EdgeCase,
    |ctx| {
        let cases = [
            "abc-123",     // hyphen
            "abc_123",     // underscore
            "abc.123",     // dot
            "abc/123",     // slash
            "abc 123",     // space
            "abc@example", // at-sign
        ];
        for id in &cases {
            let result = ContainerId::new((*id).to_string());
            ctx.assert_err(
                result,
                &format!("ContainerId::new rejects special-char ID: {id:?}"),
            );
        }
        ctx.result()
    }
}

crate::conformance_test! {
    name: "container_id_valid_alphanumeric_accepted",
    adapter: "container_id",
    category: Unit,
    |ctx| {
        let cases = [
            "abc123",
            "ABC123",
            "a",
            "deadbeef01234567",
            "DeadBeef01234567",
        ];
        for id in &cases {
            let result = ContainerId::new((*id).to_string());
            ctx.assert_ok(
                result,
                &format!("ContainerId::new accepts valid alphanumeric ID: {id:?}"),
            );
        }
        ctx.result()
    }
}
