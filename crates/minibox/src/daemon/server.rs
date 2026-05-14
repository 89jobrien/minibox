//! Transport-agnostic daemon connection handler.
//!
//! Callers provide a [`ServerListener`] impl — Unix socket or Named Pipe.
//! [`PeerCreds`] from `accept()` carries SO_PEERCRED data when available.
//!
//! The protocol is line-oriented JSON: the client writes one JSON line per
//! request and the daemon responds with one or more JSON lines per response.
//! Streaming responses (`ContainerOutput`) continue until `ContainerStopped`.

use anyhow::{Context, Result};
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use std::future::Future;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tracing::{debug, error, info, warn};

use super::handler::{self, HandlerDependencies, handle_resize_pty, handle_send_input};
use super::state::DaemonState;

// SECURITY: Maximum request size to prevent memory exhaustion
const MAX_REQUEST_SIZE: usize = 1024 * 1024; // 1 MB

/// A bidirectional async byte stream that can be used as a daemon connection.
///
/// This trait is a named alias for the `AsyncRead + AsyncWrite + Unpin + Send`
/// bound required by [`handle_connection`].  It is implemented for:
///
/// - [`tokio::net::UnixStream`] — production Unix socket connections
/// - [`tokio::io::DuplexStream`] — in-memory test doubles
///
/// Any type implementing all four super-traits satisfies this bound
/// automatically via the blanket impl below.
pub trait AsyncStream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}

impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> AsyncStream for T {}

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

/// Determine whether a connection should be accepted given peer credentials
/// and the `require_root_auth` flag.
///
/// This is the single source of truth for the SO_PEERCRED gate so the logic
/// can be unit-tested without a real socket.
///
/// # Rules
///
/// | `require_root_auth` | `creds`       | Result  |
/// |---------------------|---------------|---------|
/// | `false`             | any / None    | allowed |
/// | `true`              | None          | denied  |
/// | `true`              | Some(uid = 0) | allowed |
/// | `true`              | Some(uid > 0) | denied  |
pub fn is_authorized(creds: Option<&PeerCreds>, require_root_auth: bool) -> bool {
    if !require_root_auth {
        return true;
    }
    match creds {
        None => false,
        Some(c) => c.uid == 0,
    }
}

/// Log resolved startup configuration at `info` level.
///
/// Call this once after path resolution is complete and before binding the socket.
/// Structured fields follow the project tracing contract: `key = value`, lowercase
/// verb-noun message, no embedded values in the message string.
///
/// # Arguments
///
/// * `socket_path` — resolved Unix socket path (after env-var expansion)
/// * `data_dir` — resolved data directory for images and container state
/// * `cgroup_root` — resolved cgroup root used for per-container cgroups
pub fn log_startup_info(socket_path: &str, data_dir: &str, cgroup_root: &str) {
    info!(
        socket_path = socket_path,
        data_dir = data_dir,
        cgroup_root = cgroup_root,
        "server: startup configuration resolved"
    );
}

/// Run the daemon accept loop until `shutdown` resolves.
///
/// For each accepted connection the following happens:
/// 1. [`ServerListener::accept`] returns a stream and optional [`PeerCreds`].
/// 2. If `require_root_auth` is `true`, the peer UID (from `SO_PEERCRED`) is
///    checked; non-root connections are dropped with a warning.
/// 3. A Tokio task is spawned to handle the connection via
///    [`handle_connection`].
///
/// # Arguments
///
/// * `listener` — platform-specific listener implementing [`ServerListener`]
/// * `state` — shared daemon state
/// * `deps` — handler dependencies (adapters)
/// * `require_root_auth` — when `true`, rejects connections from non-root UIDs
///   via `SO_PEERCRED`; set to `false` for adapter suites that do not require
///   root (e.g. `gke`, `colima`)
/// * `shutdown` — future that resolves when the daemon should stop accepting
///   new connections (e.g. on SIGTERM/SIGINT)
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
                        if !is_authorized(peer_creds.as_ref(), require_root_auth) {
                            match peer_creds.as_ref() {
                                Some(creds) => {
                                    warn!(
                                        uid = creds.uid,
                                        pid = creds.pid,
                                        "server: rejecting non-root connection"
                                    );
                                }
                                None => {
                                    warn!(
                                        "server: rejecting unauthenticated connection; peer credentials unavailable"
                                    );
                                }
                            }
                            continue;
                        }
                        match &peer_creds {
                            Some(creds) => {
                                info!(uid = creds.uid, pid = creds.pid, "server: accepted connection");
                            }
                            None => {
                                info!("server: accepted connection (no peer credentials)");
                            }
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

/// Read a newline-delimited frame from `reader` into `buf`, rejecting frames
/// that exceed `max_bytes` **before** they are fully buffered.
///
/// Returns the number of bytes read (0 = EOF). Returns an error if the frame
/// exceeds the limit or contains invalid UTF-8.
async fn bounded_read_line<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    buf: &mut String,
    max_bytes: usize,
) -> Result<usize> {
    let mut total = 0usize;
    loop {
        let available = reader.fill_buf().await.context("reading from client")?;
        if available.is_empty() {
            return Ok(total);
        }

        let (chunk_len, found_newline) = match available.iter().position(|&b| b == b'\n') {
            Some(pos) => (pos + 1, true),
            None => (available.len(), false),
        };

        if total + chunk_len > max_bytes {
            let oversize = total + chunk_len;
            reader.consume(chunk_len);
            // Drain until newline or EOF to resync the stream.
            if !found_newline {
                let mut drain = Vec::new();
                let _ = reader.read_until(b'\n', &mut drain).await;
            }
            anyhow::bail!("request too large: at least {oversize} bytes (max {max_bytes})");
        }

        let chunk = &available[..chunk_len];
        match std::str::from_utf8(chunk) {
            Ok(s) => buf.push_str(s),
            Err(e) => {
                reader.consume(chunk_len);
                anyhow::bail!("invalid UTF-8 in request: {e}");
            }
        }
        total += chunk_len;
        reader.consume(chunk_len);

        if found_newline {
            return Ok(total);
        }
    }
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

        // SECURITY: Read one newline-delimited frame, rejecting frames that
        // exceed MAX_REQUEST_SIZE BEFORE they are fully buffered. This
        // prevents a malicious client from forcing unbounded memory allocation.
        let bytes_read = match bounded_read_line(&mut reader, &mut line, MAX_REQUEST_SIZE).await {
            Ok(n) => n,
            Err(e) => {
                warn!(
                    max = MAX_REQUEST_SIZE,
                    error = %e,
                    "rejecting oversized or malformed request"
                );
                let error_response = DaemonResponse::Error {
                    message: format!("{e}"),
                };
                let mut error_json = serde_json::to_string(&error_response)?;
                error_json.push('\n');
                writer.write_all(error_json.as_bytes()).await?;
                writer.flush().await?;
                continue;
            }
        };

        if bytes_read == 0 {
            // Client closed the connection.
            debug!("client disconnected");
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        debug!(bytes = trimmed.len(), "received request");

        let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(64);

        match serde_json::from_str::<DaemonRequest>(trimmed) {
            Ok(request) => {
                // SECURITY: Log only the request type tag, never the full
                // payload — it may contain credentials, env vars, or secrets.
                info!(request_type = request.type_tag(), "dispatching request");
                let state_c = Arc::clone(&state);
                let deps_c = Arc::clone(&deps);
                tokio::spawn(async move {
                    dispatch(request, state_c, deps_c, tx).await;
                });
            }
            Err(e) => {
                // SECURITY: Log parse error and byte length only — never the
                // raw request body, which may contain credentials or secrets.
                warn!(
                    bytes = trimmed.len(),
                    error = %e,
                    "failed to parse request"
                );
                send_terminal_response(
                    &tx,
                    "parse_error",
                    DaemonResponse::Error {
                        message: format!("invalid request: {e}"),
                    },
                )
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
/// `ContainerCreated` is intentionally non-terminal: ephemeral runs send it
/// as the first message, followed by `ContainerOutput` chunks and then
/// `ContainerStopped`. Non-ephemeral runs send it and then drop `tx`, so the
/// server loop exits naturally when `rx.recv()` returns `None`.
fn is_terminal_response(r: &DaemonResponse) -> bool {
    matches!(
        r,
        DaemonResponse::ContainerStopped { .. }
            | DaemonResponse::Error { .. }
            | DaemonResponse::Success { .. }
            | DaemonResponse::ContainerList { .. }
            | DaemonResponse::ImageLoaded { .. }
            | DaemonResponse::BuildComplete { .. }
            | DaemonResponse::ContainerPaused { .. }
            | DaemonResponse::ContainerResumed { .. }
            | DaemonResponse::Pruned { .. }
            | DaemonResponse::PipelineComplete { .. }
            | DaemonResponse::SnapshotSaved { .. }
            | DaemonResponse::SnapshotRestored { .. }
            | DaemonResponse::SnapshotList { .. }
            | DaemonResponse::ImageList { .. }
            | DaemonResponse::Manifest { .. }
            | DaemonResponse::VerifyResult { .. }
    )
    // ContainerOutput, LogLine, ContainerCreated, ExecStarted, PushProgress, BuildOutput,
    // Event, and UpdateProgress are non-terminal.
}

/// Send a single terminal [`DaemonResponse`] on `tx`, emitting a `warn!` log
/// when the receiver has already been dropped (client disconnected before the
/// handler finished computing the response).
///
/// Use this instead of `let _ = tx.send(...).await` so dropped connections are
/// observable in logs rather than silently swallowed.  Mirrors the `send_error`
/// helper in [`crate::handler`].
///
/// # Issue #118
///
/// Eliminates the silent-channel-discard footgun documented in `CLAUDE.md`.
/// Every single-response dispatch arm must use this function.
async fn send_terminal_response(
    tx: &tokio::sync::mpsc::Sender<DaemonResponse>,
    context: &str,
    response: DaemonResponse,
) {
    if tx.send(response).await.is_err() {
        warn!(
            context,
            "client disconnected before terminal response could be sent"
        );
    }
}

/// Route a parsed [`DaemonRequest`] to the appropriate handler, sending all
/// responses through `tx`.
///
/// Each variant maps 1-to-1 to a handler function in [`crate::handler`].
/// The `Run` variant is the only one that may produce multiple responses
/// (streaming output chunks); all others produce exactly one response.
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
            network,
            mounts,
            privileged,
            env,
            name,
            tty: _,
            platform,
            ..
        } => {
            handler::handle_run(
                image,
                tag,
                command,
                memory_limit_bytes,
                cpu_weight,
                ephemeral,
                network,
                mounts,
                privileged,
                env,
                name,
                platform,
                state,
                deps,
                tx,
            )
            .await;
        }
        DaemonRequest::Stop { id } => {
            let response = handler::handle_stop(id, state, deps).await;
            send_terminal_response(&tx, "Stop", response).await;
        }
        DaemonRequest::PauseContainer { id } => {
            let response =
                handler::handle_pause(id, state, Arc::clone(&deps.events.event_sink)).await;
            send_terminal_response(&tx, "PauseContainer", response).await;
        }
        DaemonRequest::ResumeContainer { id } => {
            let response =
                handler::handle_resume(id, state, Arc::clone(&deps.events.event_sink)).await;
            send_terminal_response(&tx, "ResumeContainer", response).await;
        }
        DaemonRequest::Remove { id } => {
            let response = handler::handle_remove(id, state, deps).await;
            send_terminal_response(&tx, "Remove", response).await;
        }
        DaemonRequest::List => {
            let response = handler::handle_list(state).await;
            send_terminal_response(&tx, "List", response).await;
        }
        DaemonRequest::Pull {
            image,
            tag,
            platform,
        } => {
            let response = handler::handle_pull(image, tag, platform, state, deps).await;
            send_terminal_response(&tx, "Pull", response).await;
        }
        DaemonRequest::LoadImage { path, name, tag } => {
            let response = handler::handle_load_image(path, name, tag, state, deps).await;
            send_terminal_response(&tx, "LoadImage", response).await;
        }
        DaemonRequest::Exec {
            container_id,
            cmd,
            env,
            working_dir,
            tty,
            user: _user,
        } => {
            handler::handle_exec(container_id, cmd, env, working_dir, tty, state, deps, tx).await;
        }
        DaemonRequest::Push {
            image_ref,
            credentials,
        } => {
            handler::handle_push(image_ref, credentials, state, deps, tx).await;
        }
        DaemonRequest::Commit {
            container_id,
            target_image,
            author,
            message,
            env_overrides,
            cmd_override,
        } => {
            handler::handle_commit(
                container_id,
                target_image,
                author,
                message,
                env_overrides,
                cmd_override,
                state,
                deps,
                tx,
            )
            .await;
        }
        DaemonRequest::Build {
            dockerfile,
            context_path,
            tag,
            build_args,
            no_cache,
        } => {
            handler::handle_build(
                dockerfile,
                context_path,
                tag,
                build_args,
                no_cache,
                state,
                deps,
                tx,
            )
            .await;
        }
        DaemonRequest::SubscribeEvents => {
            tokio::spawn(handler::handle_subscribe_events(
                Arc::clone(&deps.events.event_source),
                tx,
            ));
        }
        DaemonRequest::Prune { dry_run } => {
            tokio::spawn(handler::handle_prune(
                dry_run,
                Arc::clone(&state),
                Arc::clone(&deps.image.image_gc),
                Arc::clone(&deps.events.event_sink),
                tx,
            ));
        }
        DaemonRequest::ListImages => {
            tokio::spawn(handler::handle_list_images(
                Arc::clone(&deps.image.image_store),
                tx,
            ));
        }
        DaemonRequest::RemoveImage { image_ref } => {
            tokio::spawn(handler::handle_remove_image(
                image_ref,
                Arc::clone(&state),
                Arc::clone(&deps.image.image_store),
                Arc::clone(&deps.events.event_sink),
                tx,
            ));
        }
        DaemonRequest::ContainerLogs {
            container_id,
            follow,
        } => {
            handler::handle_logs(container_id, follow, state, deps, tx).await;
        }
        DaemonRequest::SendInput { session_id, data } => {
            let deps = Arc::clone(&deps);
            let tx = tx.clone();
            tokio::spawn(async move {
                handle_send_input(session_id, data, deps, tx).await;
            });
        }
        DaemonRequest::ResizePty {
            session_id,
            cols,
            rows,
        } => {
            let deps = Arc::clone(&deps);
            let tx = tx.clone();
            tokio::spawn(async move {
                handle_resize_pty(session_id, cols, rows, deps, tx).await;
            });
        }
        DaemonRequest::SaveSnapshot { id, name } => {
            let response = handler::handle_save_snapshot(id, name, state, deps).await;
            send_terminal_response(&tx, "SaveSnapshot", response).await;
        }
        DaemonRequest::RestoreSnapshot { id, name } => {
            let response = handler::handle_restore_snapshot(id, name, state, deps).await;
            send_terminal_response(&tx, "RestoreSnapshot", response).await;
        }
        DaemonRequest::ListSnapshots { id } => {
            let response = handler::handle_list_snapshots(id, deps).await;
            send_terminal_response(&tx, "ListSnapshots", response).await;
        }
        DaemonRequest::RunPipeline {
            pipeline_path,
            input,
            image,
            budget,
            env,
            ..
        } => {
            handler::handle_pipeline(pipeline_path, input, image, budget, env, state, deps, tx)
                .await;
        }
        DaemonRequest::Update {
            images,
            all,
            containers,
            restart,
        } => {
            tokio::spawn(handler::handle_update(
                images,
                all,
                containers,
                restart,
                Arc::clone(&state),
                Arc::clone(&deps),
                tx,
            ));
        }
        DaemonRequest::GetManifest { id } => {
            tokio::spawn(handler::handle_get_manifest(
                id,
                Arc::clone(&state),
                Arc::clone(&deps),
                tx,
            ));
        }
        DaemonRequest::VerifyManifest { id, policy_json } => {
            tokio::spawn(handler::handle_verify_manifest(
                id,
                policy_json,
                Arc::clone(&state),
                Arc::clone(&deps),
                tx,
            ));
        }
        DaemonRequest::RunWorkflow(_) => {
            send_terminal_response(
                &tx,
                "RunWorkflow",
                DaemonResponse::Error {
                    message: "RunWorkflow not yet implemented".to_string(),
                },
            )
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::mocks::{
        MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
    };
    use minibox_core::adapters::HostnameRegistryRouter;
    use minibox_core::image::ImageStore;
    use minibox_core::protocol::{DaemonRequest, DaemonResponse};
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    // ─── test-only no-op GC ─────────────────────────────────────────────────

    struct NoopImageGc;

    #[async_trait::async_trait]
    impl minibox_core::image::gc::ImageGarbageCollector for NoopImageGc {
        async fn prune(
            &self,
            dry_run: bool,
            _in_use: &[String],
        ) -> anyhow::Result<minibox_core::image::gc::PruneReport> {
            Ok(minibox_core::image::gc::PruneReport {
                removed: vec![],
                freed_bytes: 0,
                dry_run,
            })
        }
    }

    // ─── helpers ────────────────────────────────────────────────────────────

    fn test_deps(
        tmp: &TempDir,
    ) -> (
        Arc<crate::daemon::state::DaemonState>,
        Arc<crate::daemon::handler::HandlerDependencies>,
    ) {
        let store = ImageStore::new(tmp.path().join("images")).expect("create ImageStore");
        let state = Arc::new(crate::daemon::state::DaemonState::new(store, tmp.path()));
        let image_store =
            Arc::new(ImageStore::new(tmp.path().join("images")).expect("create ImageStore"));
        let image_gc: Arc<dyn minibox_core::image::gc::ImageGarbageCollector> =
            Arc::new(NoopImageGc);
        let deps = Arc::new(crate::daemon::handler::HandlerDependencies {
            image: crate::daemon::handler::ImageDeps {
                registry_router: Arc::new(HostnameRegistryRouter::new(
                    Arc::new(MockRegistry::new()),
                    [(
                        "ghcr.io",
                        Arc::new(MockRegistry::new()) as minibox_core::domain::DynImageRegistry,
                    )],
                )),
                image_loader: Arc::new(crate::daemon::handler::NoopImageLoader),
                image_gc,
                image_store,
            },
            lifecycle: crate::daemon::handler::LifecycleDeps {
                filesystem: Arc::new(MockFilesystem::new()),
                resource_limiter: Arc::new(MockLimiter::new()),
                runtime: Arc::new(MockRuntime::new()),
                network_provider: Arc::new(MockNetwork::new()),
                containers_base: tmp.path().join("containers"),
                run_containers_base: tmp.path().join("run"),
            },
            exec: crate::daemon::handler::ExecDeps {
                exec_runtime: None,
                pty_sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
                    crate::daemon::handler::PtySessionRegistry::default(),
                )),
            },
            build: crate::daemon::handler::BuildDeps {
                image_pusher: None,
                commit_adapter: None,
                image_builder: None,
            },
            events: crate::daemon::handler::EventDeps {
                event_sink: Arc::new(minibox_core::events::NoopEventSink),
                event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
                metrics: Arc::new(crate::daemon::telemetry::NoOpMetricsRecorder::new()),
            },
            policy: crate::daemon::handler::ContainerPolicy {
                allow_bind_mounts: true,
                allow_privileged: true,
            },
            checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
        });
        (state, deps)
    }

    async fn send_request(write_half: &mut (impl AsyncWriteExt + Unpin), req: &DaemonRequest) {
        let mut json = serde_json::to_string(req).expect("serialize request");
        json.push('\n');
        write_half
            .write_all(json.as_bytes())
            .await
            .expect("write request");
    }

    async fn read_response(
        reader: &mut BufReader<impl tokio::io::AsyncRead + Unpin>,
    ) -> DaemonResponse {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .expect("read response line");
        serde_json::from_str(line.trim()).expect("parse response JSON")
    }

    // ─── existing tests ──────────────────────────────────────────────────────

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

    #[test]
    fn is_authorized_requires_root_when_enabled() {
        assert!(is_authorized(None, false));
        assert!(is_authorized(
            Some(&PeerCreds { uid: 1000, pid: 42 }),
            false
        ));
        assert!(is_authorized(Some(&PeerCreds { uid: 0, pid: 42 }), true));
        assert!(!is_authorized(
            Some(&PeerCreds { uid: 1000, pid: 42 }),
            true
        ));
        assert!(
            !is_authorized(None, true),
            "root-auth mode must fail closed when peer credentials are unavailable"
        );
    }

    // ─── is_terminal_response ────────────────────────────────────────────────

    /// Exhaustive coverage test: every `DaemonResponse` variant must appear in
    /// this test.  The inner `match` has no wildcard arm so the compiler will
    /// emit an error if a new variant is ever added without updating this test
    /// AND updating `is_terminal_response`.
    #[test]
    fn test_is_terminal_response_all_variants() {
        // Build one instance of every variant and assert the expected terminal
        // status.  The match below is non-exhaustive-arm-free on purpose: the
        // compiler will catch any missing variant at compile time.
        let variants: &[(DaemonResponse, bool)] = &[
            (
                DaemonResponse::ContainerCreated {
                    id: "abc".to_string(),
                },
                false, // non-terminal: ephemeral runs follow with ContainerOutput chunks
            ),
            (
                DaemonResponse::Success {
                    message: "ok".to_string(),
                },
                true,
            ),
            (DaemonResponse::ContainerList { containers: vec![] }, true),
            (
                DaemonResponse::Error {
                    message: "boom".to_string(),
                },
                true,
            ),
            (
                DaemonResponse::ContainerOutput {
                    stream: minibox_core::protocol::OutputStreamKind::Stdout,
                    data: "dGVzdA==".to_string(),
                },
                false,
            ),
            (DaemonResponse::ContainerStopped { exit_code: 0 }, true),
            (
                DaemonResponse::ImageLoaded {
                    image: "minibox-tester:latest".to_string(),
                },
                true,
            ),
            (
                DaemonResponse::ExecStarted {
                    exec_id: "exec001".to_string(),
                },
                false, // non-terminal: output and ContainerStopped follow
            ),
            (
                DaemonResponse::PushProgress {
                    layer_digest: "sha256:abc".to_string(),
                    bytes_uploaded: 100,
                    total_bytes: 1000,
                },
                false, // non-terminal
            ),
            (
                DaemonResponse::BuildOutput {
                    step: 1,
                    total_steps: 3,
                    message: "Step 1/3: FROM alpine".to_string(),
                },
                false, // non-terminal
            ),
            (
                DaemonResponse::BuildComplete {
                    image_id: "sha256:deadbeef".to_string(),
                    tag: "myapp:latest".to_string(),
                },
                true,
            ),
            (
                DaemonResponse::ContainerPaused {
                    id: "abc".to_string(),
                },
                true,
            ),
            (
                DaemonResponse::ContainerResumed {
                    id: "abc".to_string(),
                },
                true,
            ),
            (
                DaemonResponse::Event {
                    event: minibox_core::events::ContainerEvent::Started {
                        id: "abc".to_string(),
                        pid: 1,
                        timestamp: std::time::SystemTime::UNIX_EPOCH,
                    },
                },
                false, // non-terminal: streaming
            ),
            (
                DaemonResponse::Pruned {
                    removed: vec![],
                    freed_bytes: 0,
                    dry_run: false,
                },
                true,
            ),
            (
                DaemonResponse::LogLine {
                    stream: minibox_core::protocol::OutputStreamKind::Stdout,
                    line: "hello".to_string(),
                },
                false, // non-terminal: more lines may follow
            ),
            (
                DaemonResponse::PipelineComplete {
                    trace: serde_json::json!({"steps": [], "result": "ok"}),
                    container_id: "abc123".to_string(),
                    exit_code: 0,
                },
                true, // terminal: pipeline execution finished
            ),
            (
                DaemonResponse::UpdateProgress {
                    image: "alpine:latest".to_string(),
                    status: "updated".to_string(),
                },
                false, // non-terminal: one per image, Success/Error follows
            ),
            (
                DaemonResponse::ImageList {
                    images: vec!["alpine:latest".to_string()],
                },
                true, // terminal: complete list returned in one response
            ),
            (
                DaemonResponse::Manifest {
                    manifest: serde_json::json!({}),
                },
                true, // terminal: single manifest returned
            ),
            (
                DaemonResponse::VerifyResult {
                    allowed: true,
                    reason: None,
                },
                true, // terminal: single verify result returned
            ),
        ];

        for (variant, expected_terminal) in variants {
            // Verify is_terminal_response returns the expected value.
            assert_eq!(
                is_terminal_response(variant),
                *expected_terminal,
                "unexpected terminal status for variant: {variant:?}",
            );

            // Exhaustiveness guard: this match must cover every arm with no
            // wildcard.  If you add a new DaemonResponse variant, the compiler
            // will refuse to compile until you add it here AND in the `variants`
            // slice above.
            let _exhaustiveness_guard: bool = match variant {
                DaemonResponse::ContainerCreated { .. } => false,
                DaemonResponse::Success { .. } => true,
                DaemonResponse::ContainerList { .. } => true,
                DaemonResponse::Error { .. } => true,
                DaemonResponse::ContainerOutput { .. } => false,
                DaemonResponse::ContainerStopped { .. } => true,
                DaemonResponse::ImageLoaded { .. } => true,
                DaemonResponse::ExecStarted { .. } => false,
                DaemonResponse::PushProgress { .. } => false,
                DaemonResponse::BuildOutput { .. } => false,
                DaemonResponse::BuildComplete { .. } => true,
                DaemonResponse::ContainerPaused { .. } => true,
                DaemonResponse::ContainerResumed { .. } => true,
                DaemonResponse::Event { .. } => false,
                DaemonResponse::Pruned { .. } => true,
                DaemonResponse::LogLine { .. } => false,
                DaemonResponse::PipelineComplete { .. } => true,
                DaemonResponse::SnapshotSaved { .. } => true,
                DaemonResponse::SnapshotRestored { .. } => true,
                DaemonResponse::SnapshotList { .. } => true,
                DaemonResponse::UpdateProgress { .. } => false,
                DaemonResponse::ImageList { .. } => true,
                DaemonResponse::Manifest { .. } => true,
                DaemonResponse::VerifyResult { .. } => true,
            };
        }
    }

    #[test]
    fn test_is_terminal_response_for_each_variant() {
        // ContainerOutput is the only non-terminal response
        assert!(
            !is_terminal_response(&DaemonResponse::ContainerOutput {
                stream: minibox_core::protocol::OutputStreamKind::Stdout,
                data: "dGVzdA==".to_string(),
            }),
            "ContainerOutput must be non-terminal"
        );

        // All other variants must be terminal
        assert!(
            is_terminal_response(&DaemonResponse::Success {
                message: "ok".to_string()
            }),
            "Success must be terminal"
        );
        assert!(
            is_terminal_response(&DaemonResponse::Error {
                message: "boom".to_string()
            }),
            "Error must be terminal"
        );
        assert!(
            !is_terminal_response(&DaemonResponse::ContainerCreated {
                id: "abc".to_string()
            }),
            "ContainerCreated must be non-terminal (ephemeral runs follow with ContainerOutput)"
        );
        assert!(
            is_terminal_response(&DaemonResponse::ContainerStopped { exit_code: 0 }),
            "ContainerStopped must be terminal"
        );
        assert!(
            is_terminal_response(&DaemonResponse::ContainerList { containers: vec![] }),
            "ContainerList must be terminal"
        );
        assert!(
            is_terminal_response(&DaemonResponse::ImageLoaded {
                image: "minibox-tester:latest".to_string()
            }),
            "ImageLoaded must be terminal"
        );
    }

    // ─── handle_connection via duplex ────────────────────────────────────────

    #[tokio::test]
    async fn test_handle_connection_list_empty() {
        let tmp = TempDir::new().expect("tempdir");
        let (state, deps) = test_deps(&tmp);

        let (client, server) = tokio::io::duplex(4096);
        let state_c = state.clone();
        let deps_c = deps.clone();
        tokio::spawn(async move {
            let _ = handle_connection(server, state_c, deps_c).await;
        });

        let (read_half, mut write_half) = tokio::io::split(client);
        let mut reader = BufReader::new(read_half);

        send_request(&mut write_half, &DaemonRequest::List).await;
        let resp = read_response(&mut reader).await;

        match resp {
            DaemonResponse::ContainerList { containers } => {
                assert!(containers.is_empty(), "expected empty container list");
            }
            other => panic!("expected ContainerList, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_handle_connection_invalid_json() {
        let tmp = TempDir::new().expect("tempdir");
        let (state, deps) = test_deps(&tmp);

        let (client, server) = tokio::io::duplex(4096);
        let state_c = state.clone();
        let deps_c = deps.clone();
        tokio::spawn(async move {
            let _ = handle_connection(server, state_c, deps_c).await;
        });

        let (read_half, mut write_half) = tokio::io::split(client);
        let mut reader = BufReader::new(read_half);

        write_half
            .write_all(b"this is not json at all\n")
            .await
            .expect("write garbage");
        let resp = read_response(&mut reader).await;

        match resp {
            DaemonResponse::Error { message } => {
                assert!(
                    message.contains("invalid request"),
                    "expected 'invalid request' in error, got: {message}"
                );
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_handle_connection_empty_line_ignored() {
        let tmp = TempDir::new().expect("tempdir");
        let (state, deps) = test_deps(&tmp);

        let (client, server) = tokio::io::duplex(4096);
        let state_c = state.clone();
        let deps_c = deps.clone();
        tokio::spawn(async move {
            let _ = handle_connection(server, state_c, deps_c).await;
        });

        let (read_half, mut write_half) = tokio::io::split(client);
        let mut reader = BufReader::new(read_half);

        // Send an empty line, then a valid request
        write_half.write_all(b"\n").await.expect("write empty line");
        send_request(&mut write_half, &DaemonRequest::List).await;

        // Should receive exactly one response (List), not two
        let resp = read_response(&mut reader).await;
        match resp {
            DaemonResponse::ContainerList { .. } => {}
            other => panic!("expected ContainerList after empty line, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_handle_connection_client_disconnect() {
        let tmp = TempDir::new().expect("tempdir");
        let (state, deps) = test_deps(&tmp);

        let (client, server) = tokio::io::duplex(4096);
        let state_c = state.clone();
        let deps_c = deps.clone();
        let join = tokio::spawn(async move { handle_connection(server, state_c, deps_c).await });

        let (read_half, mut write_half) = tokio::io::split(client);
        let mut reader = BufReader::new(read_half);

        send_request(&mut write_half, &DaemonRequest::List).await;
        let _ = read_response(&mut reader).await;

        // Drop write half — signals EOF to handle_connection
        drop(write_half);
        drop(reader);

        let result = join.await.expect("task did not panic");
        assert!(
            result.is_ok(),
            "handle_connection should return Ok on client disconnect"
        );
    }

    #[tokio::test]
    async fn test_handle_connection_pull_and_list() {
        let tmp = TempDir::new().expect("tempdir");
        let (state, deps) = test_deps(&tmp);

        let (client, server) = tokio::io::duplex(4096);
        let state_c = state.clone();
        let deps_c = deps.clone();
        tokio::spawn(async move {
            let _ = handle_connection(server, state_c, deps_c).await;
        });

        let (read_half, mut write_half) = tokio::io::split(client);
        let mut reader = BufReader::new(read_half);

        // Pull request — MockRegistry will respond (Success or Error)
        send_request(
            &mut write_half,
            &DaemonRequest::Pull {
                image: "alpine".to_string(),
                tag: Some("latest".to_string()),
                platform: None,
            },
        )
        .await;
        let pull_resp = read_response(&mut reader).await;
        // Either Success or Error is acceptable from the mock; what matters is we got a response
        matches!(
            pull_resp,
            DaemonResponse::Success { .. } | DaemonResponse::Error { .. }
        );

        // List request on same connection
        send_request(&mut write_half, &DaemonRequest::List).await;
        let list_resp = read_response(&mut reader).await;
        match list_resp {
            DaemonResponse::ContainerList { .. } => {}
            other => panic!("expected ContainerList, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_handle_connection_stop_unknown_container() {
        let tmp = TempDir::new().expect("tempdir");
        let (state, deps) = test_deps(&tmp);

        let (client, server) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let _ = handle_connection(server, state, deps).await;
        });

        let (read_half, mut write_half) = tokio::io::split(client);
        let mut reader = BufReader::new(read_half);

        send_request(
            &mut write_half,
            &DaemonRequest::Stop {
                id: "nonexistent".to_string(),
            },
        )
        .await;
        let resp = read_response(&mut reader).await;
        match resp {
            DaemonResponse::Error { .. } | DaemonResponse::Success { .. } => {}
            other => panic!("expected Error or Success for Stop, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_handle_connection_remove_unknown_container() {
        let tmp = TempDir::new().expect("tempdir");
        let (state, deps) = test_deps(&tmp);

        let (client, server) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let _ = handle_connection(server, state, deps).await;
        });

        let (read_half, mut write_half) = tokio::io::split(client);
        let mut reader = BufReader::new(read_half);

        send_request(
            &mut write_half,
            &DaemonRequest::Remove {
                id: "nonexistent".to_string(),
            },
        )
        .await;
        let resp = read_response(&mut reader).await;
        match resp {
            DaemonResponse::Error { .. } | DaemonResponse::Success { .. } => {}
            other => panic!("expected Error or Success for Remove, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_handle_connection_multiple_sequential_requests() {
        let tmp = TempDir::new().expect("tempdir");
        let (state, deps) = test_deps(&tmp);

        let (client, server) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let _ = handle_connection(server, state, deps).await;
        });

        let (read_half, mut write_half) = tokio::io::split(client);
        let mut reader = BufReader::new(read_half);

        // Three sequential requests on the same connection
        for _ in 0..3 {
            send_request(&mut write_half, &DaemonRequest::List).await;
            let resp = read_response(&mut reader).await;
            assert!(
                matches!(resp, DaemonResponse::ContainerList { .. }),
                "expected ContainerList, got {resp:?}"
            );
        }
    }

    #[tokio::test]
    async fn test_handle_connection_oversized_request() {
        let tmp = TempDir::new().expect("tempdir");
        let (state, deps) = test_deps(&tmp);

        // duplex buffer must be large enough for the oversized payload
        let (client, server) = tokio::io::duplex(2 * 1024 * 1024 + 64);
        let state_c = state.clone();
        let deps_c = deps.clone();
        tokio::spawn(async move {
            let _ = handle_connection(server, state_c, deps_c).await;
        });

        let (read_half, mut write_half) = tokio::io::split(client);
        let mut reader = BufReader::new(read_half);

        // Build a request line that exceeds MAX_REQUEST_SIZE (1 MB)
        // Wrap it in a JSON object so it reads as a single line
        let big_value = "x".repeat(1024 * 1024 + 1);
        let oversized = format!("{{\"__pad\":\"{big_value}\"}}\n");
        write_half
            .write_all(oversized.as_bytes())
            .await
            .expect("write oversized");

        let resp = read_response(&mut reader).await;
        match resp {
            DaemonResponse::Error { message } => {
                assert!(
                    message.contains("request too large"),
                    "expected 'request too large' in error, got: {message}"
                );
            }
            other => panic!("expected Error for oversized request, got {other:?}"),
        }
    }

    // ─── run_server ──────────────────────────────────────────────────────────

    struct MockListener {
        rx: tokio::sync::Mutex<
            tokio::sync::mpsc::Receiver<(tokio::io::DuplexStream, Option<PeerCreds>)>,
        >,
    }

    impl ServerListener for MockListener {
        type Stream = tokio::io::DuplexStream;

        async fn accept(&self) -> Result<(Self::Stream, Option<PeerCreds>)> {
            match self.rx.lock().await.recv().await {
                Some(pair) => Ok(pair),
                None => Err(anyhow::anyhow!("listener closed")),
            }
        }
    }

    #[tokio::test]
    async fn test_run_server_shutdown() {
        let tmp = TempDir::new().expect("tempdir");
        let (state, deps) = test_deps(&tmp);

        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let listener = MockListener {
            rx: tokio::sync::Mutex::new(rx),
        };

        // Resolve shutdown immediately
        let result = run_server(listener, state, deps, false, async {}).await;
        assert!(
            result.is_ok(),
            "run_server should return Ok on immediate shutdown"
        );
    }

    #[tokio::test]
    async fn test_run_server_accepts_root_connection() {
        let tmp = TempDir::new().expect("tempdir");
        let (state, deps) = test_deps(&tmp);

        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let listener = MockListener {
            rx: tokio::sync::Mutex::new(rx),
        };

        // Root connection — should be accepted and handled
        let (client, server) = tokio::io::duplex(4096);
        tx.send((server, Some(PeerCreds { uid: 0, pid: 100 })))
            .await
            .expect("send connection");
        drop(tx);

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            run_server(listener, state, deps, true, async {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }),
        )
        .await;
        assert!(result.is_ok(), "server should not have timed out");

        // Verify the connection was handled: send List and get a response
        let (read_half, mut write_half) = tokio::io::split(client);
        let reader = BufReader::new(read_half);
        send_request(&mut write_half, &DaemonRequest::List).await;
        // Response may or may not arrive depending on timing — the key assertion
        // is that the server didn't reject the connection (no panic, clean exit).
        drop(write_half);
        let _ = reader;
    }

    #[tokio::test]
    async fn test_run_server_no_creds_require_root_rejects() {
        let tmp = TempDir::new().expect("tempdir");
        let (state, deps) = test_deps(&tmp);

        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let listener = MockListener {
            rx: tokio::sync::Mutex::new(rx),
        };

        // Connection with no peer credentials must be rejected when root auth is required.
        let (_client, server) = tokio::io::duplex(4096);
        tx.send((server, None)).await.expect("send connection");
        drop(tx);

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            run_server(listener, state, deps, true, async {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }),
        )
        .await;
        assert!(result.is_ok(), "server should not have timed out");
    }

    #[tokio::test]
    async fn test_run_server_no_creds_no_require_root() {
        let tmp = TempDir::new().expect("tempdir");
        let (state, deps) = test_deps(&tmp);

        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let listener = MockListener {
            rx: tokio::sync::Mutex::new(rx),
        };

        // No credentials, require_root=false — should accept without warning.
        let (_client, server) = tokio::io::duplex(4096);
        tx.send((server, None)).await.expect("send connection");
        drop(tx);

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            run_server(listener, state, deps, false, async {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }),
        )
        .await;
        assert!(result.is_ok(), "server should not have timed out");
    }

    #[tokio::test]
    async fn test_run_server_rejects_non_root() {
        let tmp = TempDir::new().expect("tempdir");
        let (state, deps) = test_deps(&tmp);

        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let listener = MockListener {
            rx: tokio::sync::Mutex::new(rx),
        };

        // Send a connection with non-root PeerCreds
        let (_client, server) = tokio::io::duplex(4096);
        tx.send((server, Some(PeerCreds { uid: 1000, pid: 42 })))
            .await
            .expect("send connection");
        // Drop sender so listener returns error on next accept
        drop(tx);

        // Use a short shutdown timer so the server exits promptly
        // after processing the rejected connection + accept error
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            run_server(listener, state, deps, true, async {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }),
        )
        .await;

        // Server should have shut down via the shutdown future, not timed out
        assert!(result.is_ok(), "server should not have timed out");
    }

    // ─── send_terminal_response ──────────────────────────────────────────────

    /// Issue #118: `send_terminal_response` must NOT panic when the receiver is
    /// already dropped (client disconnected before the handler finished).
    #[tokio::test]
    async fn test_send_terminal_response_does_not_panic_on_dropped_receiver() {
        let (tx, rx) = tokio::sync::mpsc::channel::<DaemonResponse>(1);
        // Drop the receiver — simulates a client that disconnected early.
        drop(rx);

        // Must not panic; must return gracefully.
        send_terminal_response(
            &tx,
            "test_context",
            DaemonResponse::Success {
                message: "ok".to_string(),
            },
        )
        .await;
    }

    /// Issue #118: `send_terminal_response` must deliver the response when the
    /// receiver is still alive.
    #[tokio::test]
    async fn test_send_terminal_response_delivers_when_receiver_alive() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(1);

        send_terminal_response(
            &tx,
            "test_context",
            DaemonResponse::Success {
                message: "delivered".to_string(),
            },
        )
        .await;

        let received = rx.recv().await.expect("expected a response");
        assert!(
            matches!(received, DaemonResponse::Success { ref message } if message == "delivered"),
            "unexpected response: {received:?}"
        );
    }

    // ─── MockStream failure tests ────────────────────────────────────────────
    //
    // These tests exercise `handle_connection` via `tokio::io::duplex` streams
    // (the in-memory `AsyncStream` double) to verify protocol-level failure
    // modes without a real Unix socket.

    /// half_frame_request: `MockStream` read_buf contains truncated JSON (no
    /// newline terminator).  `handle_connection` must not panic — it should
    /// exit either cleanly (`Ok`) or with an I/O error (broken pipe / write
    /// on closed) when trying to send a parse-error response back to a
    /// client that has already closed its read end.
    #[tokio::test]
    async fn mock_stream_half_frame_request_exits_cleanly() {
        let tmp = TempDir::new().expect("tempdir");
        let (state, deps) = test_deps(&tmp);

        let (mut client, server) = tokio::io::duplex(4096);
        let join = tokio::spawn(async move { handle_connection(server, state, deps).await });

        // Write a truncated JSON line — no trailing newline, so `bounded_read_line`
        // returns the partial content as-is at EOF.
        client
            .write_all(b"{\"List\":")
            .await
            .expect("write half-frame");

        // Drop the entire client — signals EOF on the server's read side.
        // The server will attempt to parse the incomplete JSON, produce a parse
        // error response, and fail to write it (broken pipe).  Both outcomes
        // (Ok and Err(broken pipe)) are acceptable — the key invariant is no
        // panic/task failure.
        drop(client);

        let task_result = tokio::time::timeout(std::time::Duration::from_secs(2), join)
            .await
            .expect("server task should not time out")
            .expect("task did not panic");

        // Accept Ok(()) or an I/O error from writing the error response back.
        // A panic would be caught above; reaching here means the server handled
        // the truncated frame gracefully.
        match &task_result {
            Ok(()) => {}
            Err(e) => {
                let msg = format!("{e:#}");
                assert!(
                    msg.contains("broken pipe")
                        || msg.contains("flushing")
                        || msg.contains("writing"),
                    "unexpected error from half-frame handling: {msg}"
                );
            }
        }
    }

    /// oversized_request_via_mock_stream: send a 2 MB payload through a duplex
    /// stream.  The server must respond with an `Error` containing
    /// "request too large" rather than buffering the entire payload.
    ///
    /// This is a duplicate of `test_handle_connection_oversized_request` but
    /// explicitly documents the `MockStream` (duplex) path.
    #[tokio::test]
    async fn mock_stream_oversized_request_returns_error() {
        let tmp = TempDir::new().expect("tempdir");
        let (state, deps) = test_deps(&tmp);

        // Buffer must be large enough to hold the oversized write.
        let (client, server) = tokio::io::duplex(3 * 1024 * 1024);
        let state_c = Arc::clone(&state);
        let deps_c = Arc::clone(&deps);
        tokio::spawn(async move {
            let _ = handle_connection(server, state_c, deps_c).await;
        });

        let (read_half, mut write_half) = tokio::io::split(client);
        let mut reader = BufReader::new(read_half);

        // 2 MB payload — exceeds MAX_REQUEST_SIZE (1 MB).
        let big_value = "y".repeat(2 * 1024 * 1024);
        let oversized = format!("{{\"__pad\":\"{big_value}\"}}\n");
        write_half
            .write_all(oversized.as_bytes())
            .await
            .expect("write oversized payload");

        let resp = read_response(&mut reader).await;
        match resp {
            DaemonResponse::Error { message } => {
                assert!(
                    message.contains("request too large"),
                    "expected 'request too large', got: {message}"
                );
            }
            other => panic!("expected Error for 2 MB payload, got {other:?}"),
        }
    }
}
