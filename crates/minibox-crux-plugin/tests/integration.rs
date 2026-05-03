//! Integration tests for minibox-crux-plugin.
//!
//! These tests drive the binary via stdin/stdout using the cruxx plugin wire
//! protocol (newline-delimited JSON). A mock minibox daemon socket is bound in
//! each test so that `dispatch()` → `DaemonClient` calls succeed without a real
//! daemon running.
//!
//! The binary path is resolved via `CARGO_BIN_EXE_minibox-crux-plugin`, which
//! cargo sets automatically when running integration tests in the same workspace.

use minibox_core::protocol::DaemonResponse;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::process::Command;

// ── Helpers ────────────────────────────────────────────────────────────────────

fn plugin_bin() -> PathBuf {
    // Set by cargo when running integration tests in this package.
    PathBuf::from(env!("CARGO_BIN_EXE_minibox-crux-plugin"))
}

/// Spawn the plugin binary with `MINIBOX_SOCKET_PATH` pointed at `socket_path`.
/// Returns the child process with piped stdin/stdout.
fn spawn_plugin(socket_path: &Path) -> tokio::process::Child {
    Command::new(plugin_bin())
        .env("MINIBOX_SOCKET_PATH", socket_path)
        .env("RUST_LOG", "error") // suppress info logs to stderr during tests
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn minibox-crux-plugin")
}

/// Bind a mock daemon Unix socket. Accept one connection, read one request line,
/// write back `response`, then close. Intended to be tokio::spawned.
async fn mock_daemon_once(socket_path: PathBuf, response: DaemonResponse) {
    let listener = UnixListener::bind(&socket_path).expect("bind mock daemon socket");
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

/// Bind a mock daemon that accepts one connection and writes multiple responses
/// (for streaming handlers like logs).
async fn mock_daemon_multi(socket_path: PathBuf, responses: Vec<DaemonResponse>) {
    let listener = UnixListener::bind(&socket_path).expect("bind mock daemon socket");
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

/// Write a JSON line to the plugin's stdin.
async fn send(stdin: &mut tokio::process::ChildStdin, value: &Value) {
    let mut line = serde_json::to_string(value).expect("serialize request value");
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .await
        .expect("write to plugin stdin");
    stdin.flush().await.expect("flush plugin stdin");
}

/// Read one JSON line from the plugin's stdout.
async fn recv(reader: &mut BufReader<tokio::process::ChildStdout>) -> Value {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .expect("read from plugin stdout");
    serde_json::from_str(line.trim()).expect("plugin sent non-JSON")
}

// ── Tests ──────────────────────────────────────────────────────────────────────

/// `Declare` → plugin lists all 11 handlers without contacting the daemon.
#[tokio::test]
async fn declare_returns_nine_handlers() {
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("daemon.sock");

    // Declare doesn't hit the daemon — no mock needed, but socket_path must
    // exist as a valid path for MINIBOX_SOCKET_PATH.
    let mut child = spawn_plugin(&socket_path);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout_reader = BufReader::new(stdout);

    let declare_req = json!({"method": "Declare"});
    send(&mut stdin, &declare_req).await;

    let resp = recv(&mut stdout_reader).await;
    assert_eq!(resp["status"], "Declare");
    let handlers = resp["data"]["handlers"].as_array().expect("handlers array");
    assert_eq!(handlers.len(), 11);

    let names: Vec<&str> = handlers
        .iter()
        .map(|h| h["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"minibox::container::run"));
    assert!(names.contains(&"minibox::container::ps"));
    assert!(names.contains(&"minibox::image::pull"));

    // Shutdown cleanly.
    send(&mut stdin, &json!({"method": "Shutdown"})).await;
    let ack = recv(&mut stdout_reader).await;
    assert_eq!(ack["status"], "ShutdownAck");
    child.wait().await.unwrap();
}

/// `Shutdown` → plugin sends `ShutdownAck` and exits 0.
#[tokio::test]
async fn shutdown_sends_ack_and_exits() {
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("daemon.sock");

    let mut child = spawn_plugin(&socket_path);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout_reader = BufReader::new(stdout);

    send(&mut stdin, &json!({"method": "Shutdown"})).await;
    let resp = recv(&mut stdout_reader).await;
    assert_eq!(resp["status"], "ShutdownAck");

    let status = child.wait().await.unwrap();
    assert!(status.success());
}

/// Malformed JSON line → plugin skips it (no output), continues.
#[tokio::test]
async fn malformed_json_is_skipped() {
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("daemon.sock");

    let mut child = spawn_plugin(&socket_path);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout_reader = BufReader::new(stdout);

    // Send garbage, then a valid Declare to confirm the plugin is still alive.
    stdin.write_all(b"not json at all\n").await.unwrap();
    stdin.flush().await.unwrap();

    send(&mut stdin, &json!({"method": "Declare"})).await;
    let resp = recv(&mut stdout_reader).await;
    assert_eq!(resp["status"], "Declare");

    send(&mut stdin, &json!({"method": "Shutdown"})).await;
    recv(&mut stdout_reader).await; // ShutdownAck
    child.wait().await.unwrap();
}

/// Unknown handler name → `InvokeErr` (no daemon contact needed).
#[tokio::test]
async fn invoke_unknown_handler_returns_invoke_err() {
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("daemon.sock");

    let mut child = spawn_plugin(&socket_path);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout_reader = BufReader::new(stdout);

    let req = json!({
        "method": "Invoke",
        "params": {
            "handler": "minibox::unknown::handler",
            "input": {}
        }
    });
    send(&mut stdin, &req).await;
    let resp = recv(&mut stdout_reader).await;
    assert_eq!(resp["status"], "InvokeErr");
    let err = resp["data"]["error"].as_str().unwrap();
    assert!(
        err.contains("unknown handler") || err.contains("minibox::unknown::handler"),
        "error was: {err}"
    );

    send(&mut stdin, &json!({"method": "Shutdown"})).await;
    recv(&mut stdout_reader).await;
    child.wait().await.unwrap();
}

/// Missing required field on a known handler → `InvokeErr`.
#[tokio::test]
async fn invoke_missing_required_field_returns_invoke_err() {
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("daemon.sock");

    let mut child = spawn_plugin(&socket_path);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout_reader = BufReader::new(stdout);

    // `container::stop` requires "id" — omit it.
    let req = json!({
        "method": "Invoke",
        "params": {
            "handler": "minibox::container::stop",
            "input": {}
        }
    });
    send(&mut stdin, &req).await;
    let resp = recv(&mut stdout_reader).await;
    assert_eq!(resp["status"], "InvokeErr");

    send(&mut stdin, &json!({"method": "Shutdown"})).await;
    recv(&mut stdout_reader).await;
    child.wait().await.unwrap();
}

/// `container::ps` → plugin contacts daemon, returns `InvokeOk` with the list.
#[tokio::test]
async fn invoke_ps_returns_container_list() {
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("daemon.sock");

    // Spin up mock daemon before spawning plugin.
    let sp = socket_path.clone();
    tokio::spawn(async move {
        mock_daemon_once(sp, DaemonResponse::ContainerList { containers: vec![] }).await
    });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let mut child = spawn_plugin(&socket_path);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout_reader = BufReader::new(stdout);

    let req = json!({
        "method": "Invoke",
        "params": { "handler": "minibox::container::ps", "input": {} }
    });
    send(&mut stdin, &req).await;
    let resp = recv(&mut stdout_reader).await;
    assert_eq!(resp["status"], "InvokeOk");

    send(&mut stdin, &json!({"method": "Shutdown"})).await;
    recv(&mut stdout_reader).await;
    child.wait().await.unwrap();
}

/// `image::pull` → plugin contacts daemon, returns `InvokeOk` on Success.
#[tokio::test]
async fn invoke_pull_returns_success() {
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("daemon.sock");

    let sp = socket_path.clone();
    tokio::spawn(async move {
        mock_daemon_once(
            sp,
            DaemonResponse::Success {
                message: "pulled".into(),
            },
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let mut child = spawn_plugin(&socket_path);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout_reader = BufReader::new(stdout);

    let req = json!({
        "method": "Invoke",
        "params": {
            "handler": "minibox::image::pull",
            "input": { "image": "alpine" }
        }
    });
    send(&mut stdin, &req).await;
    let resp = recv(&mut stdout_reader).await;
    assert_eq!(resp["status"], "InvokeOk");

    send(&mut stdin, &json!({"method": "Shutdown"})).await;
    recv(&mut stdout_reader).await;
    child.wait().await.unwrap();
}

/// `container::stop` → plugin contacts daemon, returns `InvokeOk` on Success.
#[tokio::test]
async fn invoke_stop_returns_success() {
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("daemon.sock");

    let sp = socket_path.clone();
    tokio::spawn(async move {
        mock_daemon_once(
            sp,
            DaemonResponse::Success {
                message: "stopped".into(),
            },
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let mut child = spawn_plugin(&socket_path);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout_reader = BufReader::new(stdout);

    let req = json!({
        "method": "Invoke",
        "params": {
            "handler": "minibox::container::stop",
            "input": { "id": "abc123" }
        }
    });
    send(&mut stdin, &req).await;
    let resp = recv(&mut stdout_reader).await;
    assert_eq!(resp["status"], "InvokeOk");

    send(&mut stdin, &json!({"method": "Shutdown"})).await;
    recv(&mut stdout_reader).await;
    child.wait().await.unwrap();
}

/// Daemon connection failure → `InvokeErr` (socket does not exist).
#[tokio::test]
async fn daemon_unreachable_returns_invoke_err() {
    let tmp = TempDir::new().unwrap();
    // Deliberately do NOT bind a daemon at this path.
    let socket_path = tmp.path().join("no-daemon.sock");

    let mut child = spawn_plugin(&socket_path);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout_reader = BufReader::new(stdout);

    let req = json!({
        "method": "Invoke",
        "params": { "handler": "minibox::container::ps", "input": {} }
    });
    send(&mut stdin, &req).await;
    let resp = recv(&mut stdout_reader).await;
    assert_eq!(resp["status"], "InvokeErr");

    send(&mut stdin, &json!({"method": "Shutdown"})).await;
    recv(&mut stdout_reader).await;
    child.wait().await.unwrap();
}

/// Multiple requests in sequence — plugin handles them all before shutdown.
#[tokio::test]
async fn multiple_requests_in_sequence() {
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("daemon.sock");

    // The plugin will make two daemon calls (ps + pull) — bind two mock
    // connections by spawning two tasks that each accept one connection.
    let sp1 = socket_path.clone();
    let sp2 = socket_path.clone();

    // Bind listener; spawn a task that handles two sequential connections.
    let listener = UnixListener::bind(&socket_path).expect("bind mock daemon socket");
    tokio::spawn(async move {
        // First accept — responds to ps
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

        // Second accept — responds to pull
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

        // Drop sp1/sp2 to silence unused-variable lint.
        drop(sp1);
        drop(sp2);
    });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let mut child = spawn_plugin(&socket_path);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout_reader = BufReader::new(stdout);

    // Request 1: ps
    send(
        &mut stdin,
        &json!({
            "method": "Invoke",
            "params": { "handler": "minibox::container::ps", "input": {} }
        }),
    )
    .await;
    let r1 = recv(&mut stdout_reader).await;
    assert_eq!(r1["status"], "InvokeOk");

    // Request 2: pull
    send(
        &mut stdin,
        &json!({
            "method": "Invoke",
            "params": {
                "handler": "minibox::image::pull",
                "input": { "image": "ubuntu" }
            }
        }),
    )
    .await;
    let r2 = recv(&mut stdout_reader).await;
    assert_eq!(r2["status"], "InvokeOk");

    send(&mut stdin, &json!({"method": "Shutdown"})).await;
    let ack = recv(&mut stdout_reader).await;
    assert_eq!(ack["status"], "ShutdownAck");

    child.wait().await.unwrap();
}
