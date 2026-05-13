//! Conformance tests for the `ExecRuntime` trait contract.
//!
//! All tests use `MockExecRuntime` from `minibox::testing` — no syscalls.
//! Each test creates a fresh mock to avoid shared state.

use minibox::testing::mocks::MockExecRuntime;
use minibox_core::domain::{ContainerId, ExecRuntime, ExecSpec};
use minibox_core::protocol::DaemonResponse;
use std::any::Any;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_spec() -> ExecSpec {
    ExecSpec {
        cmd: vec!["/bin/sh".to_string()],
        env: vec![],
        working_dir: None,
        tty: false,
    }
}

fn make_container_id(s: &str) -> ContainerId {
    ContainerId::new(s.to_string()).unwrap_or_else(|e| panic!("invalid ContainerId '{s}': {e}"))
}

// ---------------------------------------------------------------------------
// Success invariants
// ---------------------------------------------------------------------------

/// A successful exec must return an `ExecHandle` with a non-empty id.
#[tokio::test]
async fn exec_success_returns_handle_with_id() {
    let runtime = MockExecRuntime::new();
    let (tx, _rx) = mpsc::channel::<DaemonResponse>(8);
    let cid = make_container_id("execconf01ab1234");

    let handle = runtime
        .run_in_container(&cid, default_spec(), tx)
        .await
        .expect("exec must succeed on default mock");

    assert!(
        !handle.id.is_empty(),
        "ExecHandle.id must be non-empty, got: {:?}",
        handle.id
    );
}

/// `call_count` must increment after a successful exec.
#[tokio::test]
async fn exec_success_increments_call_count() {
    let runtime = MockExecRuntime::new();
    let (tx, _rx) = mpsc::channel::<DaemonResponse>(8);
    let cid = make_container_id("execconf02ab1234");

    assert_eq!(runtime.call_count(), 0);
    runtime
        .run_in_container(&cid, default_spec(), tx)
        .await
        .expect("unwrap in test");
    assert_eq!(runtime.call_count(), 1);
}

/// Two successive execs must produce different handle ids.
#[tokio::test]
async fn exec_successive_calls_produce_different_ids() {
    let runtime = MockExecRuntime::new();
    let cid = make_container_id("execconf03ab1234");

    let (tx1, _) = mpsc::channel::<DaemonResponse>(8);
    let h1 = runtime
        .run_in_container(&cid, default_spec(), tx1)
        .await
        .expect("unwrap in test");

    let (tx2, _) = mpsc::channel::<DaemonResponse>(8);
    let h2 = runtime
        .run_in_container(&cid, default_spec(), tx2)
        .await
        .expect("unwrap in test");

    assert_ne!(
        h1.id, h2.id,
        "successive exec handles must have different ids"
    );
}

/// The mock captures the last `ExecSpec` for test assertions.
#[tokio::test]
async fn exec_captures_last_spec() {
    let runtime = MockExecRuntime::new();
    let cid = make_container_id("execconf04ab1234");

    let spec = ExecSpec {
        cmd: vec!["ls".to_string(), "-la".to_string()],
        env: vec!["FOO=bar".to_string()],
        working_dir: Some(PathBuf::from("/tmp")),
        tty: true,
    };

    let (tx, _) = mpsc::channel::<DaemonResponse>(8);
    runtime
        .run_in_container(&cid, spec.clone(), tx)
        .await
        .expect("unwrap in test");

    let captured = runtime
        .last_spec()
        .expect("last_spec must be Some after a call");
    assert_eq!(captured.cmd, spec.cmd);
    assert_eq!(captured.tty, true);
    assert_eq!(captured.working_dir, Some(PathBuf::from("/tmp")));
}

/// The mock captures the last container id.
#[tokio::test]
async fn exec_captures_last_container_id() {
    let runtime = MockExecRuntime::new();
    let cid = make_container_id("execconf05ab1234");

    let (tx, _) = mpsc::channel::<DaemonResponse>(8);
    runtime
        .run_in_container(&cid, default_spec(), tx)
        .await
        .expect("unwrap in test");

    assert_eq!(runtime.last_container_id().expect("must be Some"), cid);
}

// ---------------------------------------------------------------------------
// Failure invariants
// ---------------------------------------------------------------------------

/// `with_failure()` causes all subsequent execs to return Err.
#[tokio::test]
async fn exec_failure_returns_err() {
    let runtime = MockExecRuntime::new().with_failure();
    let (tx, _) = mpsc::channel::<DaemonResponse>(8);
    let cid = make_container_id("execconf06ab1234");

    let result = runtime.run_in_container(&cid, default_spec(), tx).await;
    assert!(
        result.is_err(),
        "exec must fail when configured with_failure"
    );
}

/// A failed exec still increments `call_count`.
#[tokio::test]
async fn exec_failure_increments_call_count() {
    let runtime = MockExecRuntime::new().with_failure();
    let (tx, _) = mpsc::channel::<DaemonResponse>(8);
    let cid = make_container_id("execconf07ab1234");

    let _ = runtime.run_in_container(&cid, default_spec(), tx).await;
    assert_eq!(
        runtime.call_count(),
        1,
        "call_count must increment on failure"
    );
}

// ---------------------------------------------------------------------------
// AsAny downcasting
// ---------------------------------------------------------------------------

#[test]
fn exec_runtime_downcast_succeeds() {
    let runtime = Arc::new(MockExecRuntime::new());
    let trait_obj: Arc<dyn ExecRuntime> = runtime.clone() as Arc<dyn ExecRuntime>;

    let any_ref: &dyn Any = trait_obj.as_any();
    assert!(
        any_ref.downcast_ref::<MockExecRuntime>().is_some(),
        "downcast to MockExecRuntime must succeed"
    );
}

// ---------------------------------------------------------------------------
// Default trait
// ---------------------------------------------------------------------------

#[test]
fn exec_runtime_default_creates_success_mock() {
    let runtime = MockExecRuntime::default();
    assert_eq!(runtime.call_count(), 0);
}
