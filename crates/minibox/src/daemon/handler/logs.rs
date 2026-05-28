//! Container log retrieval handler.

use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::warn;

use crate::daemon::state::DaemonState;

use super::{HandlerDependencies, send_error};

/// Retrieve stored log output for a container.
///
/// Reads `{containers_base}/{id}/stdout.log` and `stderr.log`, emitting one
/// [`DaemonResponse::LogLine`] per line.  Terminates with
/// [`DaemonResponse::Success`] when `follow` is `false` (the only supported
/// mode for now).  Sends [`DaemonResponse::Error`] when the container is not
/// found.
// qual:allow(complexity) reason: "logs handler: container lookup, file read, stream"
pub async fn handle_logs(
    name_or_id: String,
    _follow: bool,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    use anyhow::Context as _;
    use minibox_core::protocol::OutputStreamKind;
    use std::io::{BufRead, BufReader};

    let id = match state.resolve_id(&name_or_id).await {
        Some(id) => id,
        None => {
            send_error(
                &tx,
                "handle_logs",
                format!("container not found: {name_or_id}"),
            )
            .await;
            return;
        }
    };

    // Read stdout.log then stderr.log; missing files are silently skipped.
    let log_dir = deps.lifecycle.containers_base.join(&id);
    let log_pairs: &[(&str, OutputStreamKind)] = &[
        ("stdout.log", OutputStreamKind::Stdout),
        ("stderr.log", OutputStreamKind::Stderr),
    ];

    for (filename, stream) in log_pairs {
        let path = log_dir.join(filename);
        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                warn!(
                    container_id = %id,
                    path = %path.display(),
                    error = %e,
                    "handle_logs: failed to open log file"
                );
                continue;
            }
        };

        let reader = BufReader::new(file);
        for line_result in reader.lines() {
            let line = match line_result.context("reading log line") {
                Ok(l) => l,
                Err(e) => {
                    warn!(container_id = %id, error = %e, "handle_logs: read error");
                    break;
                }
            };
            if tx
                .send(DaemonResponse::LogLine {
                    stream: stream.clone(),
                    line,
                })
                .await
                .is_err()
            {
                warn!(
                    container_id = %id,
                    "handle_logs: client disconnected mid-stream"
                );
                return;
            }
        }
    }

    if tx
        .send(DaemonResponse::Success {
            message: "end of log".to_string(),
        })
        .await
        .is_err()
    {
        warn!(container_id = %id, "handle_logs: client disconnected before Success");
    }
}
