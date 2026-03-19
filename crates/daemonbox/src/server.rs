//! Transport-agnostic daemon connection handler.
//!
//! Callers provide a [`ServerListener`] impl — Unix socket or Named Pipe.
//! [`PeerCreds`] from `accept()` carries SO_PEERCRED data when available.
//!
//! The protocol is line-oriented JSON: the client writes one JSON line per
//! request and the daemon responds with one or more JSON lines per response.
//! Streaming responses (`ContainerOutput`) continue until `ContainerStopped`.

use anyhow::{Context, Result};
use minibox_lib::protocol::{DaemonRequest, DaemonResponse};
use std::future::Future;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tracing::{debug, error, info, warn};

use crate::handler::{self, HandlerDependencies};
use crate::state::DaemonState;

// SECURITY: Maximum request size to prevent memory exhaustion
const MAX_REQUEST_SIZE: usize = 1024 * 1024; // 1 MB

/// Peer credentials from an accepted connection.
#[derive(Debug, Clone)]
pub struct PeerCreds {
    pub uid: u32,
    pub pid: i32,
}

/// Platform-agnostic server listener.
///
/// Implementors wrap a platform-specific listener (Unix socket, Named Pipe, etc.)
/// and yield a stream + optional peer credentials on each `accept()` call.
pub trait ServerListener: Send + 'static {
    type Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static;

    /// Accept the next incoming connection.
    ///
    /// Returns the stream and optional peer credentials. On platforms where
    /// credential inspection is not available (e.g., Windows named pipes),
    /// `PeerCreds` may be `None`.
    fn accept(
        &self,
    ) -> impl std::future::Future<Output = Result<(Self::Stream, Option<PeerCreds>)>> + Send;
}

/// Run the daemon accept loop until `shutdown` resolves.
///
/// # Arguments
///
/// * `listener` — platform-specific listener implementing [`ServerListener`]
/// * `state` — shared daemon state
/// * `deps` — handler dependencies (adapters)
/// * `require_root_auth` — when `true`, rejects connections from non-root UIDs
/// * `shutdown` — future that resolves when the daemon should stop
pub async fn run_server<L, F>(
    listener: L,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    require_root_auth: bool,
    shutdown: F,
) -> Result<()>
where
    L: ServerListener,
    F: Future<Output = ()>,
{
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, peer_creds)) => {
                        if let Some(ref creds) = peer_creds {
                            if require_root_auth && creds.uid != 0 {
                                warn!(uid = creds.uid, pid = creds.pid, "server: rejecting non-root connection");
                                continue;
                            }
                            info!(uid = creds.uid, pid = creds.pid, "server: accepted connection");
                        } else {
                            if require_root_auth {
                                warn!("server: peer credentials unavailable; require_root_auth bypassed");
                            }
                            info!("server: accepted connection (no peer credentials)");
                        }
                        let state_clone = Arc::clone(&state);
                        let deps_clone = Arc::clone(&deps);
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, state_clone, deps_clone).await {
                                error!("connection error: {e:#}");
                            }
                        });
                    }
                    Err(e) => error!("server: accept error: {e}"),
                }
            }
            _ = &mut shutdown => {
                info!("server: shutdown signal received");
                break;
            }
        }
    }
    Ok(())
}

/// Handle a single client connection, generic over stream type.
///
/// Reads newline-delimited JSON requests, dispatches to handlers, and
/// writes newline-delimited JSON responses. Continues until the client
/// closes the connection or a fatal IO error occurs.
///
/// Streaming responses (`ContainerOutput`) are forwarded until the terminal
/// `ContainerStopped` message closes the exchange.
pub async fn handle_connection<S>(
    stream: S,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let (read_half, write_half) = tokio::io::split(stream);
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
            warn!("rejecting oversized request: {bytes_read} bytes (max {MAX_REQUEST_SIZE})");
            let error_response = DaemonResponse::Error {
                message: format!("request too large: {bytes_read} bytes (max {MAX_REQUEST_SIZE})"),
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

        let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(64);

        match serde_json::from_str::<DaemonRequest>(trimmed) {
            Ok(request) => {
                info!("dispatching request: {request:?}");
                let state_c = Arc::clone(&state);
                let deps_c = Arc::clone(&deps);
                tokio::spawn(async move {
                    dispatch(request, state_c, deps_c, tx).await;
                });
            }
            Err(e) => {
                warn!("failed to parse request '{trimmed}': {e}");
                let _ = tx
                    .send(DaemonResponse::Error {
                        message: format!("invalid request: {e}"),
                    })
                    .await;
            }
        }

        while let Some(response) = rx.recv().await {
            let terminal = is_terminal_response(&response);
            let mut response_json =
                serde_json::to_string(&response).context("serializing response")?;
            response_json.push('\n');

            debug!("sending response: {}", response_json.trim_end());

            writer
                .write_all(response_json.as_bytes())
                .await
                .context("writing response")?;
            writer.flush().await.context("flushing response")?;

            if terminal {
                break;
            }
        }
    }

    Ok(())
}

/// Returns true for response types that terminate a request/response exchange.
///
/// Non-streaming responses always terminate immediately. Streaming responses
/// (`ContainerOutput`) continue until `ContainerStopped` (which is terminal).
fn is_terminal_response(r: &DaemonResponse) -> bool {
    !matches!(r, DaemonResponse::ContainerOutput { .. })
}

/// Route a parsed `DaemonRequest` to the appropriate handler, sending
/// responses through `tx`.
async fn dispatch(
    request: DaemonRequest,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: tokio::sync::mpsc::Sender<DaemonResponse>,
) {
    match request {
        DaemonRequest::Run {
            image,
            tag,
            command,
            memory_limit_bytes,
            cpu_weight,
            ephemeral,
        } => {
            handler::handle_run(
                image,
                tag,
                command,
                memory_limit_bytes,
                cpu_weight,
                ephemeral,
                state,
                deps,
                tx,
            )
            .await;
        }
        DaemonRequest::Stop { id } => {
            let response = handler::handle_stop(id, state).await;
            let _ = tx.send(response).await;
        }
        DaemonRequest::Remove { id } => {
            let response = handler::handle_remove(id, state, deps).await;
            let _ = tx.send(response).await;
        }
        DaemonRequest::List => {
            let response = handler::handle_list(state).await;
            let _ = tx.send(response).await;
        }
        DaemonRequest::Pull { image, tag } => {
            let response = handler::handle_pull(image, tag, state, deps).await;
            let _ = tx.send(response).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_creds_fields_accessible() {
        let p = PeerCreds { uid: 1000, pid: 42 };
        assert_eq!(p.uid, 1000);
        assert_eq!(p.pid, 42);
    }

    #[test]
    fn peer_creds_clone() {
        let p = PeerCreds { uid: 0, pid: 1 };
        let q = p.clone();
        assert_eq!(q.uid, 0);
        assert_eq!(q.pid, 1);
    }
}
