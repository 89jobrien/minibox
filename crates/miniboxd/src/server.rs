//! Unix socket connection handler.
//!
//! Each accepted connection is handled by `handle_connection`.  The
//! protocol is line-oriented JSON: the client writes one JSON line per
//! request and the daemon responds with one JSON line per response.

use anyhow::{Context, Result};
use minibox_lib::protocol::{DaemonRequest, DaemonResponse};
use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
use std::os::unix::io::AsRawFd;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::UnixStream;
use tracing::{debug, info, warn};

use crate::handler;
use crate::state::DaemonState;

// SECURITY: Maximum request size to prevent memory exhaustion
const MAX_REQUEST_SIZE: usize = 1024 * 1024; // 1 MB

/// Handle a single client connection.
///
/// Reads newline-delimited JSON requests, dispatches to handlers, and
/// writes newline-delimited JSON responses.  Continues until the client
/// closes the connection or a fatal IO error occurs.
///
/// # Security
///
/// Authenticates the client via SO_PEERCRED and only accepts connections
/// from root (UID 0).
pub async fn handle_connection(stream: UnixStream, state: Arc<DaemonState>) -> Result<()> {
    // SECURITY: Get peer credentials and verify UID is root
    let raw_fd = stream.as_raw_fd();
    let creds = getsockopt(raw_fd, PeerCredentials)
        .context("failed to get peer credentials")?;

    if creds.uid() != 0 {
        warn!(
            "rejecting connection from non-root UID {} (PID {})",
            creds.uid(),
            creds.pid()
        );
        // Close connection without processing requests
        return Ok(());
    }

    info!(
        "accepted connection from UID {} PID {}",
        creds.uid(),
        creds.pid()
    );

    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = BufWriter::new(write_half);
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = reader
            .read_line(&mut line)
            .await
            .context("reading from client")?;

        if bytes_read == 0 {
            // Client closed the connection.
            debug!("client disconnected");
            break;
        }

        // SECURITY: Reject requests exceeding size limit
        if bytes_read > MAX_REQUEST_SIZE {
            warn!(
                "rejecting oversized request: {} bytes (max {})",
                bytes_read, MAX_REQUEST_SIZE
            );
            let error_response = DaemonResponse::Error {
                message: format!(
                    "request too large: {} bytes (max {})",
                    bytes_read, MAX_REQUEST_SIZE
                ),
            };
            let mut error_json = serde_json::to_string(&error_response)?;
            error_json.push('\n');
            writer.write_all(error_json.as_bytes()).await?;
            writer.flush().await?;
            continue;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        debug!("received request: {} bytes", trimmed.len());

        let response = match serde_json::from_str::<DaemonRequest>(trimmed) {
            Ok(request) => {
                info!("dispatching request: {:?}", request);
                dispatch(request, Arc::clone(&state)).await
            }
            Err(e) => {
                warn!("failed to parse request '{}': {}", trimmed, e);
                DaemonResponse::Error {
                    message: format!("invalid request: {}", e),
                }
            }
        };

        let mut response_json =
            serde_json::to_string(&response).context("serializing response")?;
        response_json.push('\n');

        debug!("sending response: {}", response_json.trim());

        writer
            .write_all(response_json.as_bytes())
            .await
            .context("writing response")?;
        writer.flush().await.context("flushing response")?;
    }

    Ok(())
}

/// Route a parsed `DaemonRequest` to the appropriate handler.
async fn dispatch(request: DaemonRequest, state: Arc<DaemonState>) -> DaemonResponse {
    match request {
        DaemonRequest::Run {
            image,
            tag,
            command,
            memory_limit_bytes,
            cpu_weight,
        } => {
            handler::handle_run(image, tag, command, memory_limit_bytes, cpu_weight, state)
                .await
        }
        DaemonRequest::Stop { id } => handler::handle_stop(id, state).await,
        DaemonRequest::Remove { id } => handler::handle_remove(id, state).await,
        DaemonRequest::List => handler::handle_list(state).await,
        DaemonRequest::Pull { image, tag } => handler::handle_pull(image, tag, state).await,
    }
}
