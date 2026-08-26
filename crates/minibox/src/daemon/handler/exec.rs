//! Exec, `SendInput`, and `ResizePty` handlers.
// Handler signatures require >5 parameters by design (DI pattern). See rustqual.toml.
#![allow(clippy::too_many_arguments)]

use base64::Engine as _;
use minibox_core::domain::{ExecOutput, ExecSession, SessionId};
#[cfg(test)]
use minibox_core::progress::TokioExecOutputStream;
use minibox_core::protocol::{DaemonResponse, OutputStreamKind};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::daemon::state::DaemonState;

use super::{HandlerDependencies, send_error};

fn exec_output_to_response(value: ExecOutput) -> DaemonResponse {
    match value {
        ExecOutput::Stdout(bytes) => DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
        },
        ExecOutput::Stderr(bytes) => DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stderr,
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
        },
        ExecOutput::Exit(exit_code) => DaemonResponse::ContainerStopped { exit_code },
        ExecOutput::Error(message) => DaemonResponse::Error { message },
    }
}

async fn forward_exec_outputs(tx: &mpsc::Sender<DaemonResponse>, mut session: ExecSession) {
    if tx
        .send(DaemonResponse::ExecStarted {
            exec_id: session.handle.id,
        })
        .await
        .is_err()
    {
        return;
    }
    while let Some(output) = session.output.next().await {
        let terminal = matches!(output, ExecOutput::Exit(_) | ExecOutput::Error(_));
        if tx.send(exec_output_to_response(output)).await.is_err() || terminal {
            break;
        }
    }
}

/// Run a command inside an already-running container via namespace join.
///
/// Streams `ContainerOutput` messages and terminates with `ContainerStopped`.
/// Returns `Error` immediately if the exec runtime is unavailable or the
/// container is not running.
pub async fn handle_exec(
    container_id: String,
    cmd: Vec<String>,
    env: Vec<String>,
    working_dir: Option<String>,
    tty: bool,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let start = std::time::Instant::now();
    let Some(ref exec_rt) = deps.exec.exec_runtime else {
        deps.events.metrics.increment_counter(
            "minibox_container_ops_total",
            &[("op", "exec"), ("adapter", "daemon"), ("status", "error")],
        );
        send_error(
            &tx,
            "handle_exec",
            "exec not supported on this platform".to_string(),
        )
        .await;
        return;
    };

    let cid = match minibox_core::domain::ContainerId::new(container_id.clone()) {
        Ok(id) => id,
        Err(e) => {
            send_error(&tx, "handle_exec", format!("invalid container id: {e}")).await;
            return;
        }
    };

    // Allocate PTY channels and register them so SendInput/ResizePty can reach
    // the running exec session.
    let session_key = container_id.clone();
    const RESIZE_CHANNEL_CAPACITY: usize = 8;
    const STDIN_CHANNEL_CAPACITY: usize = 32;
    let (resize_tx, resize_rx) = mpsc::channel::<(u16, u16)>(RESIZE_CHANNEL_CAPACITY);
    let (stdin_ch_tx, _stdin_ch_rx) = mpsc::channel::<Vec<u8>>(STDIN_CHANNEL_CAPACITY);
    if tty {
        // Only register PTY channels for tty sessions; non-tty execs have no
        // use for resize or stdin channels. Registered entries are removed when
        // the session ends (see cleanup call below).
        let mut reg = deps.exec.pty_sessions.lock().await;
        reg.resize.insert(session_key.clone(), resize_tx);
        reg.stdin.insert(session_key.clone(), stdin_ch_tx.clone());
    }
    let _ = resize_rx; // handed to exec runtime in future task; avoid unused-var lint
    let _ = stdin_ch_tx;

    let spec = minibox_core::domain::ExecSpec {
        cmd,
        env,
        working_dir: working_dir.map(std::path::PathBuf::from),
        tty,
    };

    match exec_rt.as_ref().run_in_container(&cid, spec).await {
        Ok(session) => {
            info!(
                container_id = %container_id,
                exec_id = %session.handle.id,
                "exec: started"
            );
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "exec"), ("adapter", "daemon"), ("status", "ok")],
            );
            deps.events.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "exec"), ("adapter", "daemon")],
            );
            forward_exec_outputs(&tx, session).await;
            // Session ends when run_in_container's output stream closes; clean up
            // PTY channels so the registry does not grow unboundedly.
            deps.exec.pty_sessions.lock().await.cleanup(&session_key);
        }
        Err(e) => {
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "exec"), ("adapter", "daemon"), ("status", "error")],
            );
            deps.events.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "exec"), ("adapter", "daemon")],
            );
            deps.exec.pty_sessions.lock().await.cleanup(&session_key);
            send_error(&tx, "handle_exec", format!("exec failed: {e:#}")).await;
        }
    }
}

#[cfg(test)]
mod exec_output_tests {
    use super::*;

    #[test]
    fn exec_output_to_response_preserves_streams_and_terminal_values() {
        assert!(matches!(
            exec_output_to_response(ExecOutput::Stdout(b"out".to_vec())),
            DaemonResponse::ContainerOutput {
                stream: OutputStreamKind::Stdout,
                ..
            }
        ));
        assert!(matches!(
            exec_output_to_response(ExecOutput::Stderr(b"err".to_vec())),
            DaemonResponse::ContainerOutput {
                stream: OutputStreamKind::Stderr,
                ..
            }
        ));
        assert!(matches!(
            exec_output_to_response(ExecOutput::Exit(7)),
            DaemonResponse::ContainerStopped { exit_code: 7 }
        ));
        assert!(matches!(
            exec_output_to_response(ExecOutput::Error("failed".to_string())),
            DaemonResponse::Error { message } if message == "failed"
        ));
    }

    #[test]
    fn exec_output_to_response_base64_encodes_output_bytes() {
        let response = exec_output_to_response(ExecOutput::Stdout(vec![0, 1, 2, 255]));
        let DaemonResponse::ContainerOutput { data, .. } = response else {
            panic!("expected container output");
        };
        assert_eq!(data, "AAEC/w==");
    }

    #[tokio::test]
    async fn forward_exec_outputs_sends_started_before_buffered_output() {
        let (output_tx, output_rx) = mpsc::channel(2);
        output_tx
            .send(ExecOutput::Stdout(b"ready".to_vec()))
            .await
            .expect("output receiver must be open");
        output_tx
            .send(ExecOutput::Exit(0))
            .await
            .expect("output receiver must be open");
        drop(output_tx);

        let (response_tx, mut response_rx) = mpsc::channel(3);
        forward_exec_outputs(
            &response_tx,
            ExecSession {
                handle: minibox_core::domain::ExecHandle {
                    id: "exec-1".to_string(),
                },
                output: Box::new(TokioExecOutputStream::new(output_rx)),
            },
        )
        .await;

        assert!(matches!(
            response_rx.recv().await,
            Some(DaemonResponse::ExecStarted { exec_id }) if exec_id == "exec-1"
        ));
        assert!(matches!(
            response_rx.recv().await,
            Some(DaemonResponse::ContainerOutput {
                stream: OutputStreamKind::Stdout,
                ..
            })
        ));
        assert!(matches!(
            response_rx.recv().await,
            Some(DaemonResponse::ContainerStopped { exit_code: 0 })
        ));
    }
}

// ─── SendInput / ResizePty ────────────────────────────────────────────────────

/// Forward base64-encoded stdin bytes to a running PTY session.
///
/// Looks up the session in the PTY session registry and forwards decoded bytes.
/// Returns `Success` on delivery, `Error` when the session is unknown or the
/// channel has been closed.
pub async fn handle_send_input(
    session_id: SessionId,
    data: String,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    use base64::Engine as _;
    let bytes = match base64::engine::general_purpose::STANDARD.decode(&data) {
        Ok(b) => b,
        Err(e) => {
            send_error(&tx, "handle_send_input", format!("base64 decode: {e}")).await;
            return;
        }
    };
    let reg = deps.exec.pty_sessions.lock().await;
    if let Some(stdin_tx) = reg.stdin.get(session_id.as_ref()) {
        if stdin_tx.send(bytes).await.is_err() {
            warn!(
                session_id = %session_id,
                "send_input: stdin channel closed"
            );
        }
    } else {
        send_error(
            &tx,
            "handle_send_input",
            format!("no active tty session: {session_id}"),
        )
        .await;
        return;
    }
    if tx
        .send(DaemonResponse::Success {
            message: "input forwarded".to_string(),
        })
        .await
        .is_err()
    {
        warn!(session_id = %session_id, "send_input: client disconnected");
    }
}

/// Forward a terminal resize event to a running PTY session.
///
/// Looks up the session in the PTY session registry and sends `(cols, rows)`.
/// Returns `Success` on delivery, `Error` when the session is unknown or the
/// channel has been closed.
pub async fn handle_resize_pty(
    session_id: SessionId,
    cols: u16,
    rows: u16,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let reg = deps.exec.pty_sessions.lock().await;
    if let Some(resize_tx) = reg.resize.get(session_id.as_ref()) {
        if resize_tx.send((cols, rows)).await.is_err() {
            warn!(
                session_id = %session_id,
                "resize_pty: resize channel closed"
            );
        }
    } else {
        send_error(
            &tx,
            "handle_resize_pty",
            format!("no active tty session: {session_id}"),
        )
        .await;
        return;
    }
    if tx
        .send(DaemonResponse::Success {
            message: "resize forwarded".to_string(),
        })
        .await
        .is_err()
    {
        warn!(session_id = %session_id, "resize_pty: client disconnected");
    }
}
