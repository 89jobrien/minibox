//! `KrunExecRuntime` — exec-into-running-container adapter for the krun/smolvm
//! VM backend.
//!
//! Delegates to `smolvm machine exec --name <container_id> -- <cmd>` via
//! [`KrunRuntime::exec_in_container`] — the smolvm machine name is always the
//! minibox container ID (see [`crate::krun::process`]).

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use minibox::container_state::StateHandle;
use minibox_core::domain::{
    AsAny, ContainerId, DynExecRuntime, DynProgressSink, ExecHandle, ExecOutput, ExecRuntime,
    ExecSession, ExecSpec, ProgressSink,
};
use minibox_core::progress::{TokioExecOutputStream, TokioProgressSink};
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::krun::runtime::KrunRuntime;

/// Exec adapter backed by `smolvm machine exec` against a persistent named machine.
///
/// `KrunRuntime`'s own container registry is keyed by an internally generated
/// runtime ID distinct from the daemon's `ContainerId` (see
/// [`KrunRuntime::create`]), so `state` is used to translate the daemon's
/// `ContainerId` into that runtime ID before any lookup.
pub struct KrunExecRuntime {
    runtime: Arc<KrunRuntime>,
    state: StateHandle,
}

impl KrunExecRuntime {
    /// Creates an exec runtime backed by the shared krun container registry.
    #[must_use]
    pub const fn new(runtime: Arc<KrunRuntime>, state: StateHandle) -> Self {
        Self { runtime, state }
    }
}

impl AsAny for KrunExecRuntime {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[async_trait]
impl ExecRuntime for KrunExecRuntime {
    async fn run_in_container(
        &self,
        container_id: &ContainerId,
        spec: ExecSpec,
    ) -> Result<ExecSession> {
        validate_exec_spec(&spec)?;
        let id = container_id.as_str().to_string();
        let runtime_id = self
            .state
            .get_container_runtime_id(&id)
            .await
            .with_context(|| format!("krun: no runtime id recorded for container {id}"))?;
        let env: Vec<(String, String)> = spec
            .env
            .iter()
            .filter_map(|kv| {
                kv.split_once('=')
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
            })
            .collect();

        let exec_id = Uuid::new_v4().simple().to_string();

        let mut child = self
            .runtime
            .exec_in_container(&runtime_id, &spec.cmd, &env, spec.tty)
            .await
            .with_context(|| format!("krun: exec into container {id} failed"))?;

        info!(
            container_id = %id,
            exec_id = %exec_id,
            cmd = ?spec.cmd,
            "krun exec: process started"
        );

        const EXEC_OUTPUT_CAPACITY: usize = 64;
        let (output_tx, output_rx) = mpsc::channel(EXEC_OUTPUT_CAPACITY);
        let output_sink = TokioProgressSink::shared(output_tx);
        let exec_id_for_task = exec_id.clone();
        tokio::spawn(async move {
            stream_child_output(&mut child, &exec_id_for_task, &output_sink).await;
        });

        Ok(ExecSession {
            handle: ExecHandle { id: exec_id },
            output: Box::new(TokioExecOutputStream::new(output_rx)),
        })
    }
}

/// Drain `child`'s stdout and stderr into `tx` concurrently, then send its exit code.
async fn stream_child_output(
    child: &mut tokio::process::Child,
    exec_id: &str,
    tx: &DynProgressSink<ExecOutput>,
) {
    const READ_BUFFER_SIZE: usize = 4096;

    let stdout_task = async {
        if let Some(mut out) = child.stdout.take() {
            let mut buf = [0u8; READ_BUFFER_SIZE];
            loop {
                match out.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx
                            .send(ExecOutput::Stdout(buf[..n].to_vec()))
                            .await
                            .is_err()
                        {
                            warn!(
                                exec_id,
                                "krun exec: client disconnected mid-stream (stdout)"
                            );
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(exec_id, error = %e, "krun exec: stdout read error");
                        break;
                    }
                }
            }
        }
    };
    let stderr_task = async {
        if let Some(mut err) = child.stderr.take() {
            let mut buf = [0u8; READ_BUFFER_SIZE];
            loop {
                match err.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx
                            .send(ExecOutput::Stderr(buf[..n].to_vec()))
                            .await
                            .is_err()
                        {
                            warn!(
                                exec_id,
                                "krun exec: client disconnected mid-stream (stderr)"
                            );
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(exec_id, error = %e, "krun exec: stderr read error");
                        break;
                    }
                }
            }
        }
    };
    tokio::join!(stdout_task, stderr_task);

    let exit_code = match child.wait().await {
        Ok(status) => status.code().unwrap_or(-1),
        Err(e) => {
            warn!(exec_id, error = %e, "krun exec: wait failed");
            -1
        }
    };
    let _ = tx.send(ExecOutput::Exit(exit_code)).await;
    info!(exec_id, exit_code, "krun exec: process exited");
}

fn validate_exec_spec(spec: &ExecSpec) -> Result<()> {
    let Some(program) = spec.cmd.first() else {
        return Err(anyhow!("exec command must not be empty"));
    };
    if program.is_empty() {
        return Err(anyhow!("exec command program must not be empty"));
    }
    for part in &spec.cmd {
        if part.contains('\0') {
            return Err(anyhow!("exec command arguments must not contain NUL bytes"));
        }
    }
    for kv in &spec.env {
        let Some((key, value)) = kv.split_once('=') else {
            return Err(anyhow!("exec env entries must use KEY=VALUE format"));
        };
        if key.is_empty() {
            return Err(anyhow!("exec env key must not be empty"));
        }
        if key.contains('\0') || value.contains('\0') {
            return Err(anyhow!("exec env entries must not contain NUL bytes"));
        }
    }
    Ok(())
}

/// Construct a [`DynExecRuntime`] backed by `smolvm machine exec`.
#[must_use]
pub fn krun_exec_runtime(runtime: Arc<KrunRuntime>, state: StateHandle) -> DynExecRuntime {
    Arc::new(KrunExecRuntime::new(runtime, state))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::default_constructed_unit_structs
)]
mod tests {
    use super::*;

    /// Minimal `ContainerStateAccess` stub: returns "not found" for every
    /// container, which is all `run_in_container_errors_for_unknown_container`
    /// needs to exercise the runtime-id lookup failure path.
    struct EmptyState;

    #[async_trait::async_trait]
    impl minibox::container_state::ContainerStateAccess for EmptyState {
        async fn get_container_pid(&self, container_id: &str) -> anyhow::Result<u32> {
            Err(anyhow!("container {container_id} not found"))
        }

        async fn get_container_runtime_id(&self, container_id: &str) -> anyhow::Result<String> {
            Err(anyhow!("container {container_id} not found"))
        }

        async fn get_overlay_upper(
            &self,
            container_id: &str,
        ) -> anyhow::Result<std::path::PathBuf> {
            Err(anyhow!("container {container_id} not found"))
        }

        async fn get_merged_rootfs(
            &self,
            container_id: &str,
        ) -> anyhow::Result<std::path::PathBuf> {
            Err(anyhow!("container {container_id} not found"))
        }

        async fn get_source_image_ref(&self, container_id: &str) -> anyhow::Result<String> {
            Err(anyhow!("container {container_id} not found"))
        }
    }

    fn empty_state() -> StateHandle {
        Arc::new(EmptyState)
    }

    #[test]
    fn validate_exec_spec_rejects_empty_command() {
        let spec = ExecSpec {
            cmd: vec![],
            env: vec![],
            working_dir: None,
            tty: false,
        };
        assert!(validate_exec_spec(&spec).is_err());
    }

    #[test]
    fn validate_exec_spec_rejects_invalid_env() {
        let spec = ExecSpec {
            cmd: vec!["id".to_string()],
            env: vec!["NO_SEPARATOR".to_string()],
            working_dir: None,
            tty: false,
        };
        assert!(validate_exec_spec(&spec).is_err());
    }

    #[test]
    fn validate_exec_spec_accepts_valid_spec() {
        let spec = ExecSpec {
            cmd: vec!["echo".to_string(), "hi".to_string()],
            env: vec!["KEY=VALUE".to_string()],
            working_dir: None,
            tty: false,
        };
        assert!(validate_exec_spec(&spec).is_ok());
    }

    #[tokio::test]
    async fn run_in_container_errors_for_unknown_container() {
        let runtime = Arc::new(KrunRuntime::new());
        let exec_runtime = KrunExecRuntime::new(runtime, empty_state());
        let spec = ExecSpec {
            cmd: vec!["echo".to_string(), "hi".to_string()],
            env: vec![],
            working_dir: None,
            tty: false,
        };
        let container_id = ContainerId::try_new("nonexistentid").expect("valid container id");
        let result = exec_runtime.run_in_container(&container_id, spec).await;
        assert!(result.is_err());
    }

    #[test]
    fn as_any_downcasts_to_self() {
        let runtime = Arc::new(KrunRuntime::new());
        let exec_runtime = KrunExecRuntime::new(runtime, empty_state());
        assert!(
            exec_runtime
                .as_any()
                .downcast_ref::<KrunExecRuntime>()
                .is_some()
        );
    }
}
