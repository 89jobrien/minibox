//! Mock implementation of [`ExecRuntime`].

use anyhow::Result;
use async_trait::async_trait;
use minibox_core::domain::{AsAny, ContainerId, ExecHandle, ExecRuntime, ExecSpec};
use minibox_core::protocol::DaemonResponse;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::Sender;

// ---------------------------------------------------------------------------
// MockExecRuntime
// ---------------------------------------------------------------------------

/// Mock implementation of [`ExecRuntime`] for testing.
///
/// Records calls to `run_in_container` without performing any real exec.
/// Can be configured to fail on demand. Captures the last `ExecSpec`
/// received so tests can assert on it.
#[derive(Debug, Clone)]
pub struct MockExecRuntime {
    state: Arc<Mutex<MockExecRuntimeState>>,
}

#[derive(Debug)]
struct MockExecRuntimeState {
    /// Whether `run_in_container` should return an error.
    should_fail: bool,
    /// Running count of `run_in_container` invocations.
    call_count: usize,
    /// The most recent `ExecSpec` passed to `run_in_container`.
    last_spec: Option<ExecSpec>,
    /// The most recent container ID passed to `run_in_container`.
    last_container_id: Option<ContainerId>,
}

impl MockExecRuntime {
    /// Create a new mock exec runtime that succeeds by default.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockExecRuntimeState {
                should_fail: false,
                call_count: 0,
                last_spec: None,
                last_container_id: None,
            })),
        }
    }

    /// Configure all subsequent `run_in_container` calls to return an error.
    pub fn with_failure(self) -> Self {
        self.state.lock().unwrap().should_fail = true;
        self
    }

    /// Return the total number of `run_in_container` invocations.
    pub fn call_count(&self) -> usize {
        self.state.lock().unwrap().call_count
    }

    /// Return a clone of the last `ExecSpec` received, or `None` if never called.
    pub fn last_spec(&self) -> Option<ExecSpec> {
        self.state.lock().unwrap().last_spec.clone()
    }

    /// Return a clone of the last container ID received, or `None` if never called.
    pub fn last_container_id(&self) -> Option<ContainerId> {
        self.state.lock().unwrap().last_container_id.clone()
    }
}

#[async_trait]
impl ExecRuntime for MockExecRuntime {
    /// Simulate exec by recording the call and returning a fake handle.
    ///
    /// Does not send any `DaemonResponse` messages — the `tx` is accepted
    /// but not used. Tests that need to verify output streaming should inject
    /// a real channel and drive it separately.
    async fn run_in_container(
        &self,
        container_id: &ContainerId,
        spec: ExecSpec,
        _tx: Sender<DaemonResponse>,
    ) -> Result<ExecHandle> {
        let mut state = self.state.lock().unwrap();
        state.call_count += 1;
        state.last_container_id = Some(container_id.clone());
        state.last_spec = Some(spec.clone());

        if state.should_fail {
            anyhow::bail!("mock exec failure");
        }

        Ok(ExecHandle {
            id: format!("mock-exec-{}", state.call_count),
        })
    }
}

impl AsAny for MockExecRuntime {
    fn as_any(&self) -> &dyn ::std::any::Any {
        self
    }
}

impl Default for MockExecRuntime {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn records_call_and_returns_handle() {
        let mock = MockExecRuntime::new();
        let (tx, _rx) = mpsc::channel(8);
        let id = ContainerId::new("c1abc123".to_string()).unwrap();
        let spec = ExecSpec {
            cmd: vec!["sh".into()],
            env: vec![],
            working_dir: None,
            tty: false,
        };

        let handle = mock.run_in_container(&id, spec.clone(), tx).await.unwrap();
        assert_eq!(mock.call_count(), 1);
        assert_eq!(mock.last_container_id().unwrap(), id);
        assert_eq!(mock.last_spec().unwrap().cmd, spec.cmd);
        assert!(handle.id.contains("mock-exec"));
    }

    #[tokio::test]
    async fn with_failure_returns_error() {
        let mock = MockExecRuntime::new().with_failure();
        let (tx, _rx) = mpsc::channel(8);
        let id = ContainerId::new("c2abc123".to_string()).unwrap();
        let spec = ExecSpec {
            cmd: vec!["sh".into()],
            env: vec![],
            working_dir: None,
            tty: false,
        };

        assert!(mock.run_in_container(&id, spec, tx).await.is_err());
        assert_eq!(mock.call_count(), 1);
    }

    #[tokio::test]
    async fn call_count_increments_on_failure() {
        let mock = MockExecRuntime::new().with_failure();
        let (tx, _rx) = mpsc::channel(8);
        let id = ContainerId::new("c3abc123".to_string()).unwrap();
        let spec = ExecSpec {
            cmd: vec!["ls".into()],
            env: vec![],
            working_dir: None,
            tty: false,
        };
        let _ = mock.run_in_container(&id, spec, tx).await;
        assert_eq!(mock.call_count(), 1);
    }
}
