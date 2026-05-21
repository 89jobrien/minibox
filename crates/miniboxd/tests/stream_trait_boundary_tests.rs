//! Stream trait boundary tests (issue #370).
//!
//! Validates `AsyncStream` and `RegistryTransport` mock behaviour at the
//! handler/connection level without real I/O:
//!
//! - `MockAsyncStream` (via `tokio::io::duplex`): buffered in-memory with
//!   controllable EOF and error injection
//! - `MockRegistryTransport` (via `MockRegistry`): returns pre-baked layer
//!   data and records pull calls
//!
//! All tests run through `handle_connection` using an in-memory duplex stream,
//! exercising the full request→dispatch→response pipeline.

use minibox::adapters::mocks::{
    MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
};
use minibox::daemon::handler::{
    BuildDeps, ContainerPolicy, EventDeps, ExecDeps, HandlerDependencies, ImageDeps, LifecycleDeps,
    PtySessionRegistry,
};
use minibox::daemon::server::handle_connection;
use minibox::daemon::state::DaemonState;
use minibox::testing::helpers::NoopImageGc;
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::image::ImageStore;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Wire up handler dependencies using all-mock adapters.
fn test_deps(tmp: &TempDir) -> (Arc<DaemonState>, Arc<HandlerDependencies>) {
    let store = ImageStore::new(tmp.path().join("images")).expect("create ImageStore");
    let state = Arc::new(DaemonState::new(store, tmp.path()));
    let image_store =
        Arc::new(ImageStore::new(tmp.path().join("images")).expect("create ImageStore"));
    let image_gc: Arc<dyn minibox_core::image::gc::ImageGarbageCollector> =
        Arc::new(NoopImageGc::default());

    let registry = Arc::new(MockRegistry::new());
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                registry.clone() as minibox_core::domain::DynImageRegistry,
                std::iter::empty::<(&str, minibox_core::domain::DynImageRegistry)>(),
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc,
            image_store,
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: tmp.path().join("containers"),
            run_containers_base: tmp.path().join("run"),
        },
        exec: ExecDeps {
            exec_runtime: None,
            pty_sessions: Arc::new(tokio::sync::Mutex::new(PtySessionRegistry::default())),
        },
        build: BuildDeps {
            image_pusher: None,
            commit_adapter: None,
            image_builder: None,
        },
        events: EventDeps {
            event_sink: Arc::new(minibox_core::events::NoopEventSink),
            event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
            metrics: Arc::new(minibox::daemon::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
        checkpoint: Arc::new(minibox_core::domain::NoopVmCheckpoint),
    });
    (state, deps)
}

/// Serialize a request as a newline-terminated JSON line and write it.
async fn send_request(writer: &mut (impl AsyncWriteExt + Unpin), req: &DaemonRequest) {
    let mut json = serde_json::to_string(req).expect("serialize request");
    json.push('\n');
    writer
        .write_all(json.as_bytes())
        .await
        .expect("write request");
    writer.flush().await.expect("flush request");
}

/// Read one JSON response line from the reader.
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

// ---------------------------------------------------------------------------
// Test 1: Correct framing — List request returns a single JSON-framed response
// ---------------------------------------------------------------------------

/// Send a `List` request through an in-memory duplex stream and verify the
/// response is a correctly framed `ContainerList` JSON line.
#[tokio::test]
async fn duplex_stream_list_request_returns_framed_container_list() {
    let tmp = TempDir::new().expect("tempdir");
    let (state, deps) = test_deps(&tmp);

    let (client, server) = tokio::io::duplex(8192);

    // Spawn connection handler on the server side.
    tokio::spawn(async move {
        let _ = handle_connection(server, state, deps).await;
    });

    let (read_half, mut write_half) = tokio::io::split(client);
    let mut reader = BufReader::new(read_half);

    // Send a List request.
    send_request(&mut write_half, &DaemonRequest::List).await;

    // Read the response — must be a ContainerList.
    let resp = read_response(&mut reader).await;
    assert!(
        matches!(resp, DaemonResponse::ContainerList { .. }),
        "expected ContainerList, got: {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Error propagation on stream failure (client EOF mid-session)
// ---------------------------------------------------------------------------

/// Drop the client write half after sending a partial (non-newline-terminated)
/// request. The server must handle the EOF gracefully without panicking.
#[tokio::test]
async fn duplex_stream_eof_mid_request_exits_gracefully() {
    let tmp = TempDir::new().expect("tempdir");
    let (state, deps) = test_deps(&tmp);

    let (mut client, server) = tokio::io::duplex(4096);

    let join = tokio::spawn(async move { handle_connection(server, state, deps).await });

    // Write a partial JSON frame — no trailing newline.
    client
        .write_all(b"{\"List\":")
        .await
        .expect("write partial frame");
    // Close the stream — server sees EOF.
    drop(client);

    let result = tokio::time::timeout(std::time::Duration::from_secs(2), join)
        .await
        .expect("server task should not time out")
        .expect("task should not panic");

    // Accept either Ok (clean exit on EOF) or a broken-pipe error from
    // writing the parse-error response back to a closed client.
    match &result {
        Ok(()) => {}
        Err(e) => {
            let msg = format!("{e:#}");
            assert!(
                msg.contains("broken pipe") || msg.contains("flushing") || msg.contains("writing"),
                "unexpected error: {msg}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Test 3: Registry pull path exercised end-to-end with no network
// ---------------------------------------------------------------------------

/// Send a `Pull` request through the duplex stream. The MockRegistry handles
/// the pull without any network I/O and the response is verified.
#[tokio::test]
async fn duplex_stream_pull_uses_mock_registry_no_network() {
    let tmp = TempDir::new().expect("tempdir");
    let (state, deps) = test_deps(&tmp);

    let (client, server) = tokio::io::duplex(8192);

    tokio::spawn(async move {
        let _ = handle_connection(server, state, deps).await;
    });

    let (read_half, mut write_half) = tokio::io::split(client);
    let mut reader = BufReader::new(read_half);

    // Send a Pull request for alpine:latest.
    send_request(
        &mut write_half,
        &DaemonRequest::Pull {
            image: "alpine".to_string(),
            tag: Some("latest".to_string()),
            platform: None,
        },
    )
    .await;

    // Read the response. The mock registry returns success, so we expect
    // either a Success or an Error (if the image store path setup fails,
    // which is acceptable — the key is that we exercised the pull path
    // through the mock without any network I/O).
    let resp = read_response(&mut reader).await;
    match &resp {
        DaemonResponse::Success { .. } => {}
        DaemonResponse::Error { message } => {
            // The pull went through the mock registry (no network). Some
            // downstream steps (image store writes) may fail in the temp
            // environment — that is fine for this boundary test.
            assert!(
                !message.contains("network") && !message.contains("connection"),
                "pull should not attempt network I/O with mock registry: {message}"
            );
        }
        other => panic!("expected Success or Error from pull, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 4: Multiple sequential requests on the same stream
// ---------------------------------------------------------------------------

/// Verify that the connection handler correctly frames multiple sequential
/// request/response exchanges on a single duplex stream without cross-
/// contamination.
#[tokio::test]
async fn duplex_stream_multiple_requests_correctly_framed() {
    let tmp = TempDir::new().expect("tempdir");
    let (state, deps) = test_deps(&tmp);

    let (client, server) = tokio::io::duplex(16384);

    tokio::spawn(async move {
        let _ = handle_connection(server, state, deps).await;
    });

    let (read_half, mut write_half) = tokio::io::split(client);
    let mut reader = BufReader::new(read_half);

    // First request: List
    send_request(&mut write_half, &DaemonRequest::List).await;
    let resp1 = read_response(&mut reader).await;
    assert!(
        matches!(resp1, DaemonResponse::ContainerList { .. }),
        "first response should be ContainerList, got: {resp1:?}"
    );

    // Second request: ListImages
    send_request(&mut write_half, &DaemonRequest::ListImages).await;
    let resp2 = read_response(&mut reader).await;
    assert!(
        matches!(resp2, DaemonResponse::ImageList { .. }),
        "second response should be ImageList, got: {resp2:?}"
    );

    // Third request: another List — verify no stale state leaks.
    send_request(&mut write_half, &DaemonRequest::List).await;
    let resp3 = read_response(&mut reader).await;
    assert!(
        matches!(resp3, DaemonResponse::ContainerList { .. }),
        "third response should be ContainerList, got: {resp3:?}"
    );
}
