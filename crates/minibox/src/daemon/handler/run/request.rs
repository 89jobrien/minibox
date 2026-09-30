//! Request and streaming boundaries for container runs.

use minibox_core::events::ContainerEvent;
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, instrument, warn};

use crate::daemon::network_lifecycle::NetworkLifecycle;
use crate::daemon::state::DaemonState;

use super::super::{HandlerDependencies, send_error};
#[cfg(unix)]
use super::run_inner_capture;
use super::{RunParams, check_oom_killed, run_inner};

/// Create and start a new container from `image:tag`, executing `command`.
///
/// Responses are sent via `tx`. Non-ephemeral runs send exactly one message.
/// Ephemeral runs (Linux-only) send zero or more `ContainerOutput` messages
/// followed by one terminal `ContainerStopped` message.
// qual:allow(iosp) reason: "handler orchestration — validate, create, start, stream"
#[instrument(skip(params, state, deps, tx), fields(image = %params.image, ephemeral = params.ephemeral))]
pub async fn handle_run(
    params: RunParams,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let nesting = crate::nesting::NestingContext::from_env();
    if let Err(error) = nesting.check_depth() {
        let message = format!("handle_run: {error}");
        warn!(message = %message, "handle_run: nesting depth exceeded");
        if tx.send(DaemonResponse::Error { message }).await.is_err() {
            warn!("handle_run: client disconnected before depth error could be sent");
        }
        return;
    }

    let effective_policy = params
        .policy_override
        .as_ref()
        .map_or_else(|| deps.policy.clone(), |ov| deps.policy.with_overrides(ov));
    if let Err(message) = super::super::validate_policy(
        &params.mounts,
        params.privileged,
        params.priority,
        &effective_policy,
    ) {
        warn!(message = %message, "handle_run: policy violation");
        if tx.send(DaemonResponse::Error { message }).await.is_err() {
            warn!("handle_run: client disconnected before policy error could be sent");
        }
        return;
    }

    #[allow(clippy::collapsible_if)]
    if let Some(ref name) = params.name {
        if state.name_in_use(name).await {
            send_error(
                &tx,
                "handle_run",
                format!("container name {name:?} is already in use"),
            )
            .await;
            return;
        }
    }

    #[cfg(unix)]
    if params.ephemeral {
        handle_run_streaming(params, state, deps, tx).await;
        return;
    }

    let response = match run_inner(params, state, deps).await {
        Ok(id) => DaemonResponse::ContainerCreated { id },
        Err(error) => {
            error!("handle_run error: {error:#}");
            DaemonResponse::Error {
                message: format!("{error:#}"),
            }
        }
    };
    if tx.send(response).await.is_err() {
        warn!("handle_run: client disconnected before response could be sent");
    }
}

/// Stream an ephemeral container's output followed by its exit status.
// qual:allow(iosp) reason: "handler orchestration — spawn, stream, cleanup"
#[cfg(unix)]
async fn handle_run_streaming(
    params: RunParams,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    use minibox_core::protocol::OutputStreamKind;
    use std::os::fd::IntoRawFd;

    let image_label = format!(
        "{}:{}",
        params.image,
        params.tag.as_deref().unwrap_or("latest")
    );
    let result = run_inner_capture(params, Arc::clone(&state), Arc::clone(&deps)).await;

    let (container_id, pid, output_reader, runtime_id) = match result {
        Ok(result) => result,
        Err(error) => {
            error!("handle_run_streaming setup error: {error:#}");
            send_error(&tx, "handle_run", format!("{error:#}")).await;
            return;
        }
    };

    debug!(pid, "streaming: sending ContainerCreated");
    deps.events.event_sink.emit(ContainerEvent::Created {
        id: container_id.clone(),
        image: image_label,
        timestamp: std::time::SystemTime::now(),
    });
    deps.events.event_sink.emit(ContainerEvent::Started {
        id: container_id.clone(),
        pid,
        timestamp: std::time::SystemTime::now(),
    });
    let _ = tx
        .send(DaemonResponse::ContainerCreated {
            id: container_id.clone(),
        })
        .await;
    debug!(pid, "streaming: ContainerCreated sent, spawning drain");

    let tx_clone = tx.clone();
    // SAFETY: OwnedFd is consumed here and reconstructed exactly once inside the closure.
    let reader_raw = output_reader.into_raw_fd();
    let stdout_log_path = deps
        .lifecycle
        .containers_base
        .join(&container_id)
        .join("stdout.log");
    let drain_handle = tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        use std::os::fd::FromRawFd;

        // SAFETY: reader_raw is the uniquely owned descriptor transferred into this closure.
        let mut file = unsafe { std::fs::File::from_raw_fd(reader_raw) };
        let mut log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stdout_log_path)
            .map_err(|error| {
                warn!(
                    path = %stdout_log_path.display(),
                    %error,
                    "streaming: failed to open stdout.log for writing"
                );
            })
            .ok();
        const READ_BUFFER_SIZE: usize = 4096;
        let mut buffer = [0_u8; READ_BUFFER_SIZE];
        loop {
            match file.read(&mut buffer) {
                Ok(0) => break,
                Ok(bytes_read) => {
                    if let Some(ref mut log_file) = log_file
                        && let Err(error) = log_file.write_all(&buffer[..bytes_read])
                    {
                        warn!(
                            path = %stdout_log_path.display(),
                            %error,
                            "streaming: stdout.log write error"
                        );
                    }
                    use base64::Engine as _;
                    let data =
                        base64::engine::general_purpose::STANDARD.encode(&buffer[..bytes_read]);
                    let _ = tx_clone.blocking_send(DaemonResponse::ContainerOutput {
                        stream: OutputStreamKind::Stdout,
                        data,
                    });
                }
                Err(error) => {
                    warn!(pid, %error, "pipe drain: read error");
                    break;
                }
            }
        }
    });

    debug!(pid, "streaming: waiting for child exit");
    let runtime = Arc::clone(&deps.lifecycle.runtime);
    let exit_code = runtime
        .wait_for_exit(runtime_id.as_deref(), pid)
        .await
        .unwrap_or(-1);
    debug!(pid, exit_code, "streaming: child exited");

    debug!(pid, "streaming: waiting for drain");
    if let Err(error) = drain_handle.await {
        warn!(pid, %error, "pipe drain task panicked");
    }
    debug!(pid, "streaming: drain complete");

    NetworkLifecycle::new(deps.lifecycle.network_provider.clone())
        .cleanup(&container_id)
        .await;
    debug!(pid, "streaming: network cleanup done");

    let cgroup_path = state
        .get_container(&container_id)
        .await
        .map(|record| record.cgroup_path);
    state.remove_container(&container_id).await;
    debug!(pid, "streaming: container removed");

    let oom_killed = if let Some(cgroup_path) = &cgroup_path {
        check_oom_killed(cgroup_path).await
    } else {
        false
    };
    if oom_killed {
        deps.events.event_sink.emit(ContainerEvent::OomKilled {
            id: container_id.clone(),
            timestamp: std::time::SystemTime::now(),
        });
    } else {
        deps.events.event_sink.emit(ContainerEvent::Stopped {
            id: container_id.clone(),
            exit_code,
            timestamp: std::time::SystemTime::now(),
        });
    }

    let _ = tx
        .send(DaemonResponse::ContainerStopped { exit_code })
        .await;
    debug!(pid, "streaming: ContainerStopped sent");
}
