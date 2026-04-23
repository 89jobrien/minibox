//! `NoopExecRuntime` — in-memory test double (Issue #134).
//!
//! An [`crate::domain::ExecRuntime`] adapter that always returns a "not
//! supported" error.  Used to test that non-native adapters (GKE, Colima,
//! macOS VZ, WSL2) surface a clear error rather than panicking or silently
//! succeeding when `exec` is called.
//!
//! Wire this in any adapter suite that does not support in-container exec at
//! the `HandlerDependencies` composition root:
//!
//! ```rust,ignore
//! use minibox_core::adapters::NoopExecRuntime;
//! let exec: DynExecRuntime = Arc::new(NoopExecRuntime::new("gke"));
//! ```

use async_trait::async_trait;

/// An [`crate::domain::ExecRuntime`] that always returns a "not supported" error.
///
/// Non-native adapter suites (GKE, Colima, macOS VZ, WSL2) do not implement
/// in-container exec.  This noop adapter ensures that calling `exec` on those
/// suites produces a clear diagnostic error rather than a panic.
pub struct NoopExecRuntime {
    adapter_name: String,
}

crate::as_any!(NoopExecRuntime);

impl NoopExecRuntime {
    /// Create a new noop adapter that reports `adapter_name` in error messages.
    pub fn new(adapter_name: impl Into<String>) -> Self {
        Self {
            adapter_name: adapter_name.into(),
        }
    }
}

#[async_trait]
impl crate::domain::ExecRuntime for NoopExecRuntime {
    async fn run_in_container(
        &self,
        _container_id: &crate::domain::ContainerId,
        _spec: crate::domain::ExecSpec,
        _tx: tokio::sync::mpsc::Sender<crate::protocol::DaemonResponse>,
    ) -> anyhow::Result<crate::domain::ExecHandle> {
        anyhow::bail!(
            "exec is not supported on the '{}' adapter",
            self.adapter_name
        )
    }
}

#[cfg(test)]
mod tests {
    use super::NoopExecRuntime;
    use crate::domain::{ContainerId, ExecRuntime, ExecSpec};

    /// Issue #134: non-native adapters (GKE, Colima, macOS VZ) must return a
    /// clear error when `exec` is called — they must NOT panic or silently succeed.
    ///
    /// `NoopExecRuntime` is the in-memory test double for this boundary.
    #[tokio::test]
    async fn noop_exec_runtime_returns_not_supported_error() {
        let exec = NoopExecRuntime::new("gke");
        let id = ContainerId::new("testcontainerid".to_string()).unwrap();
        let spec = ExecSpec {
            cmd: vec!["/bin/sh".to_string()],
            env: vec![],
            working_dir: None,
            tty: false,
        };
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let result = exec.run_in_container(&id, spec, tx).await;

        assert!(result.is_err(), "NoopExecRuntime must always return Err");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("not supported"),
            "error must say 'not supported', got: {msg}"
        );
        assert!(
            msg.contains("gke"),
            "error must name the adapter 'gke', got: {msg}"
        );
    }

    /// The adapter name is surfaced in the error message so operators can
    /// diagnose why exec failed when switching between adapter suites.
    #[tokio::test]
    async fn noop_exec_runtime_error_includes_adapter_name() {
        for name in &["colima", "gke", "wsl2", "vz"] {
            let exec = NoopExecRuntime::new(*name);
            let (tx, _rx) = tokio::sync::mpsc::channel(8);
            let result = exec
                .run_in_container(
                    &ContainerId::new("cid1234".to_string()).unwrap(),
                    ExecSpec {
                        cmd: vec!["ls".to_string()],
                        env: vec![],
                        working_dir: None,
                        tty: false,
                    },
                    tx,
                )
                .await;

            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains(name),
                "error for adapter '{name}' must mention the adapter name; got: {msg}"
            );
        }
    }
}
