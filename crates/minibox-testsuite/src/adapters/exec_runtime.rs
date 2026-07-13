//! Conformance tests for the [`ExecRuntime`] trait contract.
//!
//! All tests use `MockExecRuntime` — no real exec is performed.

use minibox::testing::mocks::exec::MockExecRuntime;
use minibox_core::domain::{ContainerId, ExecRuntime, ExecSpec};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

fn container_id() -> ContainerId {
    ContainerId::new("cexec001abc".to_string()).expect("valid container id")
}

fn basic_spec() -> ExecSpec {
    ExecSpec {
        cmd: vec!["sh".to_string()],
        env: vec![],
        working_dir: None,
        tty: false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// `run_in_container` succeeds and returns a handle.
crate::conformance_test! {
    name: "run_in_container_returns_handle",
    adapter: "exec_runtime",
    capability: Exec,
    category: Unit,
    |ctx| {
        let mock = MockExecRuntime::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let result = rt().block_on(mock.run_in_container(
            &container_id(),
            basic_spec(),
            std::sync::Arc::new(tx),
        ));
        ctx.assert_ok(result, "run_in_container should succeed");
        ctx.result()
    }
}

// `run_in_container` increments the call count.
crate::conformance_test! {
    name: "run_in_container_increments_count",
    adapter: "exec_runtime",
    capability: Exec,
    category: Unit,
    |ctx| {
        let mock = MockExecRuntime::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let _ = rt().block_on(mock.run_in_container(
            &container_id(),
            basic_spec(),
            std::sync::Arc::new(tx),
        ));
        ctx.assert_eq(1, mock.call_count(), "call_count after one exec");
        ctx.result()
    }
}

// `run_in_container` records the last spec.
crate::conformance_test! {
    name: "run_in_container_records_spec",
    adapter: "exec_runtime",
    capability: Exec,
    category: Unit,
    |ctx| {
        let mock = MockExecRuntime::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let spec = ExecSpec {
            cmd: vec!["ls".to_string(), "-la".to_string()],
            env: vec!["HOME=/root".to_string()],
            working_dir: None,
            tty: false,
        };
        let _ =
            rt().block_on(mock.run_in_container(&container_id(), spec, std::sync::Arc::new(tx)));
        let recorded = mock.last_spec();
        ctx.assert_true(recorded.is_some(), "last_spec should be recorded");
        if let Some(s) = recorded {
            ctx.assert_eq(
                vec!["ls".to_string(), "-la".to_string()],
                s.cmd,
                "spec.cmd should match",
            );
        }
        ctx.result()
    }
}

// `run_in_container` returns Err when configured to fail.
crate::conformance_test! {
    name: "run_in_container_failure_returns_err",
    adapter: "exec_runtime",
    capability: Exec,
    category: EdgeCase,
    |ctx| {
        let mock = MockExecRuntime::new().with_failure();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let result = rt().block_on(mock.run_in_container(
            &container_id(),
            basic_spec(),
            std::sync::Arc::new(tx),
        ));
        ctx.assert_err(
            result,
            "run_in_container with failure configured must return Err",
        );
        ctx.result()
    }
}
