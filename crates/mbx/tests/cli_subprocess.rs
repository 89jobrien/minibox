//! Subprocess integration tests for the `mbx` CLI binary.
//!
//! Each test spins up a mock Unix socket server, points the binary at it via
//! `MINIBOX_SOCKET_PATH`, runs the binary as a subprocess with `assert_cmd`,
//! and asserts on exit code + stdout/stderr.
//!
//! Error paths that call `std::process::exit(1)` are testable here but not in
//! unit tests — this is the primary motivation for this test suite.

#![cfg(all(unix, feature = "subprocess-tests"))]

use assert_cmd::Command;
use minibox_core::protocol::{ContainerInfo, DaemonResponse};
use predicates::prelude::*;
use std::path::Path;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Bind a Unix socket, accept one connection, read one request line, send back
/// `response`, then close.  Spawned as a background task before running the
/// CLI subprocess.
async fn serve_once(socket_path: &Path, response: DaemonResponse) {
    let listener = UnixListener::bind(socket_path).unwrap();
    let (stream, _) = listener.accept().await.unwrap();
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let mut resp = serde_json::to_string(&response).unwrap();
    resp.push('\n');
    write_half.write_all(resp.as_bytes()).await.unwrap();
    write_half.flush().await.unwrap();
}

/// Locate the mbx binary.
///
/// Search order:
/// 1. `MINIBOX_TEST_BIN_DIR` env var (set by `just test-cli-subprocess`)
/// 2. `target/release/mbx`
/// 3. `target/debug/mbx`
fn find_minibox() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("MINIBOX_TEST_BIN_DIR") {
        let p = std::path::PathBuf::from(&dir).join("mbx");
        if p.exists() {
            return p;
        }
    }
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("could not find workspace root");
    for profile in ["release", "debug"] {
        let p = workspace_root.join("target").join(profile).join("mbx");
        if p.exists() {
            return p;
        }
    }
    panic!(
        "Could not find mbx binary. Run `cargo build -p mbx` first, \
         or set MINIBOX_TEST_BIN_DIR."
    );
}

/// Run the minibox binary with `MINIBOX_SOCKET_PATH` set to `socket_path`.
fn minibox(socket_path: &Path) -> Command {
    let mut cmd = Command::from_std(std::process::Command::new(find_minibox()));
    cmd.env("MINIBOX_SOCKET_PATH", socket_path);
    cmd
}

/// Run the minibox binary without a socket (for tests that fail before connecting).
fn minibox_no_socket() -> Command {
    Command::from_std(std::process::Command::new(find_minibox()))
}

/// Set up a temp dir + socket path, spawn `serve_once` in the background,
/// sleep briefly so the listener is bound before the subprocess connects.
async fn setup(response: DaemonResponse) -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("test.sock");
    let sp = socket_path.clone();
    tokio::spawn(async move { serve_once(&sp, response).await });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    (tmp, socket_path)
}

// ---------------------------------------------------------------------------
// ps
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ps_exits_zero_with_empty_list() {
    let (_tmp, socket_path) = setup(DaemonResponse::ContainerList { containers: vec![] }).await;

    minibox(&socket_path)
        .arg("ps")
        .assert()
        .success()
        .stdout(predicate::str::contains("CONTAINER ID"))
        .stdout(predicate::str::contains("(no containers)"));
}

#[tokio::test]
async fn ps_exits_zero_with_container_row() {
    let container = ContainerInfo {
        id: "abc123456789ab".to_string(),
        name: None,
        image: "alpine".to_string(),
        command: "/bin/sh".to_string(),
        state: "running".to_string(),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        pid: Some(42),
    };
    let (_tmp, socket_path) = setup(DaemonResponse::ContainerList {
        containers: vec![container],
    })
    .await;

    minibox(&socket_path)
        .arg("ps")
        .assert()
        .success()
        .stdout(predicate::str::contains("abc123456789ab"))
        .stdout(predicate::str::contains("alpine"))
        .stdout(predicate::str::contains("running"));
}

#[tokio::test]
async fn ps_exits_one_on_error_response() {
    let (_tmp, socket_path) = setup(DaemonResponse::Error {
        message: "daemon exploded".to_string(),
    })
    .await;

    minibox(&socket_path)
        .arg("ps")
        .assert()
        .failure()
        .stderr(predicate::str::contains("daemon exploded"));
}

// ---------------------------------------------------------------------------
// pull
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pull_exits_zero_on_success() {
    let (_tmp, socket_path) = setup(DaemonResponse::Success {
        message: "pulled alpine:latest".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["pull", "alpine"])
        .assert()
        .success()
        .stdout(predicate::str::contains("pulled alpine:latest"));
}

#[tokio::test]
async fn pull_exits_one_on_error() {
    let (_tmp, socket_path) = setup(DaemonResponse::Error {
        message: "image not found".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["pull", "doesnotexist"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("image not found"));
}

// ---------------------------------------------------------------------------
// stop
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stop_exits_zero_on_success() {
    let (_tmp, socket_path) = setup(DaemonResponse::Success {
        message: "stopped".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["stop", "abc123"])
        .assert()
        .success();
}

#[tokio::test]
async fn stop_exits_one_on_error() {
    let (_tmp, socket_path) = setup(DaemonResponse::Error {
        message: "container not found".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["stop", "nosuchcontainer"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("container not found"));
}

// ---------------------------------------------------------------------------
// rm
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rm_exits_zero_on_success() {
    let (_tmp, socket_path) = setup(DaemonResponse::Success {
        message: "removed".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["rm", "abc123"])
        .assert()
        .success();
}

#[tokio::test]
async fn rm_exits_one_on_error() {
    let (_tmp, socket_path) = setup(DaemonResponse::Error {
        message: "container still running".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["rm", "abc123"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("container still running"));
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_exits_with_container_exit_code_zero() {
    let (_tmp, socket_path) = setup(DaemonResponse::ContainerStopped { exit_code: 0 }).await;

    minibox(&socket_path)
        .args(["run", "alpine", "--", "/bin/echo", "hi"])
        .assert()
        .code(0);
}

#[tokio::test]
async fn run_exits_with_nonzero_container_exit_code() {
    let (_tmp, socket_path) = setup(DaemonResponse::ContainerStopped { exit_code: 42 }).await;

    minibox(&socket_path)
        .args(["run", "alpine", "--", "/bin/false"])
        .assert()
        .code(42);
}

#[tokio::test]
async fn run_exits_one_on_error_response() {
    let (_tmp, socket_path) = setup(DaemonResponse::Error {
        message: "image not cached".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["run", "alpine", "--", "/bin/sh"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("image not cached"));
}

#[test]
fn run_exits_one_on_unknown_network_mode() {
    // This error fires before connecting to the daemon — no socket needed.
    minibox_no_socket()
        .args(["run", "--network", "docker", "alpine", "--", "/bin/sh"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown network mode"));
}
