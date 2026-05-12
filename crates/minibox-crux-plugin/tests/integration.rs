//! Integration tests for minibox-crux-plugin.
//!
//! These tests drive the binary via stdin/stdout using the cruxx plugin wire
//! protocol (newline-delimited JSON). A mock minibox daemon socket is bound in
//! each test so that `dispatch()` -> `DaemonClient` calls succeed without a real
//! daemon running.
//!
//! The binary path is resolved via `CARGO_BIN_EXE_minibox-crux-plugin`, which
//! cargo sets automatically when running integration tests in the same workspace.

use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::process::Command;
use tokio::sync::oneshot;

// -- Helpers ------------------------------------------------------------------

fn plugin_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_minibox-crux-plugin"))
}

fn spawn_plugin(socket_path: &Path) -> tokio::process::Child {
    Command::new(plugin_bin())
        .env("MINIBOX_SOCKET_PATH", socket_path)
        .env("RUST_LOG", "error")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn minibox-crux-plugin")
}

// -- PluginHarness ------------------------------------------------------------

struct PluginHarness {
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    child: tokio::process::Child,
}

impl PluginHarness {
    fn spawn(socket_path: &Path) -> Self {
        let mut child = spawn_plugin(socket_path);
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        Self {
            stdin,
            stdout: BufReader::new(stdout),
            child,
        }
    }

    async fn send(&mut self, value: &Value) {
        let mut line = serde_json::to_string(value).expect("serialize request");
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .expect("write to plugin stdin");
        self.stdin.flush().await.expect("flush plugin stdin");
    }

    async fn send_raw(&mut self, data: &[u8]) {
        self.stdin.write_all(data).await.expect("write raw");
        self.stdin.flush().await.expect("flush raw");
    }

    async fn recv(&mut self) -> Value {
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .await
            .expect("read from plugin stdout");
        serde_json::from_str(line.trim()).expect("plugin sent non-JSON")
    }

    async fn invoke(&mut self, handler: &str, input: Value) -> Value {
        self.send(&json!({
            "method": "Invoke",
            "params": { "handler": handler, "input": input }
        }))
        .await;
        self.recv().await
    }

    async fn shutdown(mut self) -> std::process::ExitStatus {
        self.send(&json!({"method": "Shutdown"})).await;
        let ack = self.recv().await;
        assert_eq!(ack["status"], "ShutdownAck");
        self.child.wait().await.expect("child wait")
    }
}

// -- Mock daemon helpers ------------------------------------------------------

/// Accept one connection on a pre-bound listener, read one request, write one
/// response. No sleep needed -- the socket is bound before the plugin starts.
async fn mock_daemon_once(listener: UnixListener, response: DaemonResponse) {
    let (stream, _) = listener.accept().await.expect("accept mock connection");
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .expect("read request line");
    let mut resp = serde_json::to_string(&response).expect("serialize mock response");
    resp.push('\n');
    write_half
        .write_all(resp.as_bytes())
        .await
        .expect("write mock response");
    write_half.flush().await.expect("flush mock response");
}

/// Accept one connection, read one request, write multiple responses (streaming).
async fn mock_daemon_multi(listener: UnixListener, responses: Vec<DaemonResponse>) {
    let (stream, _) = listener.accept().await.expect("accept mock connection");
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .expect("read request line");
    for r in responses {
        let mut resp = serde_json::to_string(&r).expect("serialize mock response");
        resp.push('\n');
        write_half
            .write_all(resp.as_bytes())
            .await
            .expect("write mock response");
    }
    write_half.flush().await.expect("flush mock responses");
}

/// Accept one connection, capture the deserialized DaemonRequest via oneshot,
/// then write a canned response.
async fn mock_daemon_verify(
    listener: UnixListener,
    response: DaemonResponse,
    tx: oneshot::Sender<DaemonRequest>,
) {
    let (stream, _) = listener.accept().await.expect("accept mock connection");
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .expect("read request line");

    let request: DaemonRequest =
        serde_json::from_str(line.trim()).expect("deserialize DaemonRequest");
    let _ = tx.send(request);

    let mut resp = serde_json::to_string(&response).expect("serialize mock response");
    resp.push('\n');
    write_half
        .write_all(resp.as_bytes())
        .await
        .expect("write mock response");
    write_half.flush().await.expect("flush mock response");
}

/// Bind a listener and return it with the socket path. Convenience for tests.
fn bind_mock(tmp: &TempDir) -> (UnixListener, PathBuf) {
    let socket_path = tmp.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind mock daemon socket");
    (listener, socket_path)
}

// -- Tests: protocol basics ---------------------------------------------------

#[tokio::test]
async fn declare_returns_thirteen_handlers() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");
    let mut h = PluginHarness::spawn(&socket_path);

    h.send(&json!({"method": "Declare"})).await;
    let resp = h.recv().await;
    assert_eq!(resp["status"], "Declare");
    let handlers = resp["data"]["handlers"].as_array().expect("handlers array");
    assert_eq!(handlers.len(), 13);

    let names: Vec<&str> = handlers
        .iter()
        .map(|hd| hd["name"].as_str().expect("handler name"))
        .collect();
    assert!(names.contains(&"minibox::container::run"));
    assert!(names.contains(&"minibox::container::ps"));
    assert!(names.contains(&"minibox::image::pull"));

    h.shutdown().await;
}

#[tokio::test]
async fn shutdown_sends_ack_and_exits() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");
    let h = PluginHarness::spawn(&socket_path);
    let status = h.shutdown().await;
    assert!(status.success());
}

#[tokio::test]
async fn malformed_json_is_skipped() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");
    let mut h = PluginHarness::spawn(&socket_path);

    h.send_raw(b"not json at all\n").await;
    h.send(&json!({"method": "Declare"})).await;
    let resp = h.recv().await;
    assert_eq!(resp["status"], "Declare");

    h.shutdown().await;
}

// -- Tests: error paths -------------------------------------------------------

#[tokio::test]
async fn invoke_unknown_handler_returns_invoke_err() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");
    let mut h = PluginHarness::spawn(&socket_path);

    let resp = h.invoke("minibox::unknown::handler", json!({})).await;
    assert_eq!(resp["status"], "InvokeErr");
    let err = resp["data"]["error"].as_str().expect("error string");
    assert!(
        err.contains("unknown handler") || err.contains("minibox::unknown::handler"),
        "error was: {err}"
    );

    h.shutdown().await;
}

#[tokio::test]
async fn invoke_missing_required_field_returns_invoke_err() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");
    let mut h = PluginHarness::spawn(&socket_path);

    let resp = h.invoke("minibox::container::stop", json!({})).await;
    assert_eq!(resp["status"], "InvokeErr");

    h.shutdown().await;
}

#[tokio::test]
async fn daemon_unreachable_returns_invoke_err() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("no-daemon.sock");
    let mut h = PluginHarness::spawn(&socket_path);

    let resp = h.invoke("minibox::container::ps", json!({})).await;
    assert_eq!(resp["status"], "InvokeErr");

    h.shutdown().await;
}

// -- Tests: single-response handlers ------------------------------------------

#[tokio::test]
async fn invoke_ps_returns_container_list() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    tokio::spawn(mock_daemon_once(
        listener,
        DaemonResponse::ContainerList { containers: vec![] },
    ));
    let mut h = PluginHarness::spawn(&socket_path);

    let resp = h.invoke("minibox::container::ps", json!({})).await;
    assert_eq!(resp["status"], "InvokeOk");

    h.shutdown().await;
}

#[tokio::test]
async fn invoke_pull_returns_success() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    tokio::spawn(mock_daemon_once(
        listener,
        DaemonResponse::Success {
            message: "pulled".into(),
        },
    ));
    let mut h = PluginHarness::spawn(&socket_path);

    let resp = h
        .invoke("minibox::image::pull", json!({"image": "alpine"}))
        .await;
    assert_eq!(resp["status"], "InvokeOk");

    h.shutdown().await;
}

#[tokio::test]
async fn invoke_stop_returns_success() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    tokio::spawn(mock_daemon_once(
        listener,
        DaemonResponse::Success {
            message: "stopped".into(),
        },
    ));
    let mut h = PluginHarness::spawn(&socket_path);

    let resp = h
        .invoke("minibox::container::stop", json!({"id": "abc123"}))
        .await;
    assert_eq!(resp["status"], "InvokeOk");

    h.shutdown().await;
}

#[tokio::test]
async fn invoke_container_pause_sends_correct_request() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    tokio::spawn(mock_daemon_once(
        listener,
        DaemonResponse::ContainerPaused {
            id: "abc123".into(),
        },
    ));
    let mut h = PluginHarness::spawn(&socket_path);

    let resp = h
        .invoke("minibox::container::pause", json!({"id": "abc123"}))
        .await;
    assert_eq!(resp["status"], "InvokeOk");

    h.shutdown().await;
}

#[tokio::test]
async fn invoke_container_resume_sends_correct_request() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    tokio::spawn(mock_daemon_once(
        listener,
        DaemonResponse::ContainerResumed {
            id: "abc123".into(),
        },
    ));
    let mut h = PluginHarness::spawn(&socket_path);

    let resp = h
        .invoke("minibox::container::resume", json!({"id": "abc123"}))
        .await;
    assert_eq!(resp["status"], "InvokeOk");

    h.shutdown().await;
}

#[tokio::test]
async fn invoke_image_ls_sends_correct_request() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    tokio::spawn(mock_daemon_once(
        listener,
        DaemonResponse::ImageList { images: vec![] },
    ));
    let mut h = PluginHarness::spawn(&socket_path);

    let resp = h.invoke("minibox::image::ls", json!({})).await;
    assert_eq!(resp["status"], "InvokeOk");

    h.shutdown().await;
}

#[tokio::test]
async fn invoke_image_rm_sends_correct_request() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    tokio::spawn(mock_daemon_once(
        listener,
        DaemonResponse::Success {
            message: "removed".into(),
        },
    ));
    let mut h = PluginHarness::spawn(&socket_path);

    let resp = h
        .invoke("minibox::image::rm", json!({"image_ref": "alpine:latest"}))
        .await;
    assert_eq!(resp["status"], "InvokeOk");

    h.shutdown().await;
}

// -- Tests: multi-request sequence --------------------------------------------

#[tokio::test]
async fn multiple_requests_in_sequence() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind mock daemon socket");

    tokio::spawn(async move {
        // First accept -- responds to ps
        let (stream, _) = listener
            .accept()
            .await
            .expect("accept first mock connection");
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .expect("read first request");
        let mut resp = serde_json::to_string(&DaemonResponse::ContainerList { containers: vec![] })
            .expect("serialize ContainerList");
        resp.push('\n');
        write_half
            .write_all(resp.as_bytes())
            .await
            .expect("write first response");
        write_half.flush().await.expect("flush first response");

        // Second accept -- responds to pull
        let (stream2, _) = listener
            .accept()
            .await
            .expect("accept second mock connection");
        let (read_half2, mut write_half2) = tokio::io::split(stream2);
        let mut reader2 = BufReader::new(read_half2);
        let mut line2 = String::new();
        reader2
            .read_line(&mut line2)
            .await
            .expect("read second request");
        let mut resp2 = serde_json::to_string(&DaemonResponse::Success {
            message: "pulled".into(),
        })
        .expect("serialize Success");
        resp2.push('\n');
        write_half2
            .write_all(resp2.as_bytes())
            .await
            .expect("write second response");
        write_half2.flush().await.expect("flush second response");
    });

    let mut h = PluginHarness::spawn(&socket_path);

    let r1 = h.invoke("minibox::container::ps", json!({})).await;
    assert_eq!(r1["status"], "InvokeOk");

    let r2 = h
        .invoke("minibox::image::pull", json!({"image": "ubuntu"}))
        .await;
    assert_eq!(r2["status"], "InvokeOk");

    h.shutdown().await;
}

// -- Tests: request verification (#340) ---------------------------------------

#[tokio::test]
async fn invoke_ps_sends_list_request() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let (tx, rx) = oneshot::channel();
    tokio::spawn(mock_daemon_verify(
        listener,
        DaemonResponse::ContainerList { containers: vec![] },
        tx,
    ));
    let mut h = PluginHarness::spawn(&socket_path);

    let resp = h.invoke("minibox::container::ps", json!({})).await;
    assert_eq!(resp["status"], "InvokeOk");

    let req = rx.await.expect("request captured");
    assert!(
        matches!(req, DaemonRequest::List),
        "expected List, got: {req:?}"
    );

    h.shutdown().await;
}

#[tokio::test]
async fn invoke_stop_sends_correct_id() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let (tx, rx) = oneshot::channel();
    tokio::spawn(mock_daemon_verify(
        listener,
        DaemonResponse::Success {
            message: "stopped".into(),
        },
        tx,
    ));
    let mut h = PluginHarness::spawn(&socket_path);

    let resp = h
        .invoke("minibox::container::stop", json!({"id": "xyz789"}))
        .await;
    assert_eq!(resp["status"], "InvokeOk");

    let req = rx.await.expect("request captured");
    assert!(
        matches!(req, DaemonRequest::Stop { ref id } if id == "xyz789"),
        "expected Stop{{id: xyz789}}, got: {req:?}"
    );

    h.shutdown().await;
}

// -- Tests: mount round-trip (#339) -------------------------------------------

#[tokio::test]
async fn invoke_run_with_mounts_sends_correct_bind_mounts() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let (tx, rx) = oneshot::channel();
    tokio::spawn(mock_daemon_verify(
        listener,
        DaemonResponse::ContainerCreated {
            id: "test-123".into(),
        },
        tx,
    ));
    let mut h = PluginHarness::spawn(&socket_path);

    let resp = h
        .invoke(
            "minibox::container::run",
            json!({
                "image": "alpine:latest",
                "command": ["/bin/sh"],
                "mounts": [{
                    "host_path": "/tmp/data",
                    "container_path": "/data",
                    "read_only": true
                }]
            }),
        )
        .await;
    assert_eq!(resp["status"], "InvokeOk");

    let req = rx.await.expect("request captured");
    match req {
        DaemonRequest::Run { mounts, .. } => {
            assert_eq!(mounts.len(), 1, "expected 1 mount");
            assert_eq!(mounts[0].host_path, std::path::PathBuf::from("/tmp/data"));
            assert_eq!(mounts[0].container_path, std::path::PathBuf::from("/data"));
            assert!(mounts[0].read_only, "mount should be read_only");
        }
        other => panic!("expected Run, got: {other:?}"),
    }

    h.shutdown().await;
}

// -- Tests: streaming handlers (#338) -----------------------------------------

#[tokio::test]
async fn invoke_exec_returns_streaming_output() {
    use minibox_core::protocol::OutputStreamKind;

    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    tokio::spawn(mock_daemon_multi(
        listener,
        vec![
            DaemonResponse::ExecStarted {
                exec_id: "exec-1".into(),
            },
            DaemonResponse::ContainerOutput {
                stream: OutputStreamKind::Stdout,
                data: "aGVsbG8=".into(),
            },
            DaemonResponse::ContainerStopped { exit_code: 0 },
        ],
    ));
    let mut h = PluginHarness::spawn(&socket_path);

    let resp = h
        .invoke(
            "minibox::container::exec",
            json!({"id": "abc123", "command": ["/bin/echo", "hello"]}),
        )
        .await;
    assert_eq!(resp["status"], "InvokeOk");
    let output = &resp["data"]["output"];
    assert!(output.is_array(), "streaming output must be array");
    let arr = output.as_array().expect("array");
    assert_eq!(
        arr.len(),
        3,
        "ExecStarted + ContainerOutput + ContainerStopped"
    );

    h.shutdown().await;
}

#[tokio::test]
async fn invoke_build_returns_streaming_output() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    tokio::spawn(mock_daemon_multi(
        listener,
        vec![
            DaemonResponse::BuildOutput {
                step: 1,
                total_steps: 2,
                message: "Step 1/2 : FROM alpine".into(),
            },
            DaemonResponse::BuildComplete {
                image_id: "sha256:abc123".into(),
                tag: "test:latest".into(),
            },
            // BuildComplete is NOT terminal in dispatch(); Success is needed.
            DaemonResponse::Success {
                message: "build complete".into(),
            },
        ],
    ));
    let mut h = PluginHarness::spawn(&socket_path);

    let resp = h
        .invoke(
            "minibox::image::build",
            json!({"context_path": "/tmp/ctx", "tag": "test:latest"}),
        )
        .await;
    assert_eq!(resp["status"], "InvokeOk");
    let output = &resp["data"]["output"];
    assert!(output.is_array(), "streaming output must be array");
    let arr = output.as_array().expect("array");
    assert_eq!(arr.len(), 3, "BuildOutput + BuildComplete + Success");

    h.shutdown().await;
}

#[tokio::test]
async fn invoke_logs_returns_streaming_output() {
    use minibox_core::protocol::OutputStreamKind;

    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    tokio::spawn(mock_daemon_multi(
        listener,
        vec![
            DaemonResponse::ContainerOutput {
                stream: OutputStreamKind::Stdout,
                data: "bG9nIGxpbmU=".into(),
            },
            DaemonResponse::ContainerStopped { exit_code: 0 },
        ],
    ));
    let mut h = PluginHarness::spawn(&socket_path);

    let resp = h
        .invoke("minibox::container::logs", json!({"id": "abc123"}))
        .await;
    assert_eq!(resp["status"], "InvokeOk");
    let output = &resp["data"]["output"];
    assert!(output.is_array(), "streaming output must be array");
    let arr = output.as_array().expect("array");
    assert_eq!(arr.len(), 2, "ContainerOutput + ContainerStopped");

    h.shutdown().await;
}
