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

// ---------------------------------------------------------------------------
// exec
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exec_exits_zero_on_exec_started_then_close() {
    // Server sends ExecStarted then closes — execute() returns Ok(()).
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("exec_test.sock");
    let sp = socket_path.clone();
    tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixListener;
        let listener = UnixListener::bind(&sp).unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let resp = serde_json::to_string(&DaemonResponse::ExecStarted {
            exec_id: "x1".to_string(),
        })
        .unwrap();
        write_half
            .write_all(format!("{resp}\n").as_bytes())
            .await
            .unwrap();
        write_half.flush().await.unwrap();
        // Close — stream.next() returns None, execute returns Ok(()).
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    minibox(&socket_path)
        .args(["exec", "abc123", "--", "/bin/sh"])
        .assert()
        .success();
}

#[tokio::test]
async fn exec_exits_one_on_error_response() {
    let (_tmp, socket_path) = setup(DaemonResponse::Error {
        message: "container not running".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["exec", "abc123", "--", "/bin/sh"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("container not running"));
}

// ---------------------------------------------------------------------------
// logs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn logs_exits_zero_on_success() {
    // Server sends a LogLine then Success — execute() returns Ok(()).
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("logs_sub.sock");
    let sp = socket_path.clone();
    tokio::spawn(async move {
        use minibox_core::protocol::OutputStreamKind;
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixListener;
        let listener = UnixListener::bind(&sp).unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        for resp in [
            DaemonResponse::LogLine {
                stream: OutputStreamKind::Stdout,
                line: "log line one".to_string(),
            },
            DaemonResponse::Success {
                message: "end of log".to_string(),
            },
        ] {
            let mut encoded = serde_json::to_string(&resp).unwrap();
            encoded.push('\n');
            write_half.write_all(encoded.as_bytes()).await.unwrap();
        }
        write_half.flush().await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    minibox(&socket_path)
        .args(["logs", "abc123"])
        .assert()
        .success()
        .stdout(predicate::str::contains("log line one"));
}

#[tokio::test]
async fn logs_exits_one_on_error_response() {
    let (_tmp, socket_path) = setup(DaemonResponse::Error {
        message: "container not found".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["logs", "nosuchcontainer"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("container not found"));
}

// ---------------------------------------------------------------------------
// pause / resume
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pause_exits_zero_on_success() {
    let (_tmp, socket_path) = setup(DaemonResponse::Success {
        message: "paused".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["pause", "abc123"])
        .assert()
        .success();
}

#[tokio::test]
async fn pause_exits_one_on_error() {
    let (_tmp, socket_path) = setup(DaemonResponse::Error {
        message: "container not running".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["pause", "abc123"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("container not running"));
}

#[tokio::test]
async fn resume_exits_zero_on_success() {
    let (_tmp, socket_path) = setup(DaemonResponse::Success {
        message: "resumed".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["resume", "abc123"])
        .assert()
        .success();
}

#[tokio::test]
async fn resume_exits_one_on_error() {
    let (_tmp, socket_path) = setup(DaemonResponse::Error {
        message: "container not paused".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["resume", "abc123"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("container not paused"));
}

// ---------------------------------------------------------------------------
// prune
// ---------------------------------------------------------------------------

#[tokio::test]
async fn prune_exits_zero_with_removed_images() {
    let (_tmp, socket_path) = setup(DaemonResponse::Pruned {
        removed: vec!["alpine:latest".to_string(), "ubuntu:22.04".to_string()],
        freed_bytes: 52_428_800,
        dry_run: false,
    })
    .await;

    minibox(&socket_path)
        .arg("prune")
        .assert()
        .success()
        .stdout(predicate::str::contains("alpine:latest"))
        .stdout(predicate::str::contains("50.0 MB"));
}

#[tokio::test]
async fn prune_dry_run_shows_prefix() {
    let (_tmp, socket_path) = setup(DaemonResponse::Pruned {
        removed: vec!["nginx:latest".to_string()],
        freed_bytes: 10_485_760,
        dry_run: true,
    })
    .await;

    minibox(&socket_path)
        .args(["prune", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[dry-run]"));
}

#[tokio::test]
async fn prune_exits_zero_with_nothing_removed() {
    let (_tmp, socket_path) = setup(DaemonResponse::Pruned {
        removed: vec![],
        freed_bytes: 0,
        dry_run: false,
    })
    .await;

    minibox(&socket_path)
        .arg("prune")
        .assert()
        .success()
        .stdout(predicate::str::contains("0 images"));
}

#[tokio::test]
async fn prune_exits_one_on_error() {
    let (_tmp, socket_path) = setup(DaemonResponse::Error {
        message: "prune failed".to_string(),
    })
    .await;

    minibox(&socket_path)
        .arg("prune")
        .assert()
        .failure()
        .stderr(predicate::str::contains("prune failed"));
}

// ---------------------------------------------------------------------------
// rmi
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rmi_exits_zero_on_success() {
    let (_tmp, socket_path) = setup(DaemonResponse::Success {
        message: "removed alpine:latest".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["rmi", "alpine:latest"])
        .assert()
        .success()
        .stdout(predicate::str::contains("removed alpine:latest"));
}

#[tokio::test]
async fn rmi_exits_one_on_error() {
    let (_tmp, socket_path) = setup(DaemonResponse::Error {
        message: "image not found".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["rmi", "nosuchimage:latest"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("image not found"));
}

// ---------------------------------------------------------------------------
// load
// ---------------------------------------------------------------------------

#[tokio::test]
async fn load_exits_zero_on_image_loaded() {
    let (_tmp, socket_path) = setup(DaemonResponse::ImageLoaded {
        image: "myimage:latest".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["load", "--name", "myimage", "/tmp/myimage.tar"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Loaded image: myimage:latest"));
}

#[tokio::test]
async fn load_exits_one_on_error() {
    let (_tmp, socket_path) = setup(DaemonResponse::Error {
        message: "invalid tar archive".to_string(),
    })
    .await;

    minibox(&socket_path)
        .args(["load", "/tmp/broken.tar"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid tar archive"));
}

// ---------------------------------------------------------------------------
// events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn events_exits_zero_when_daemon_closes_connection() {
    // events streams until server closes — send one event then close.
    use minibox_core::events::ContainerEvent;
    use std::time::SystemTime;

    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("events_test.sock");
    let sp = socket_path.clone();
    tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixListener;
        let listener = UnixListener::bind(&sp).unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let event = ContainerEvent::Created {
            id: "ctr1".to_string(),
            image: "alpine".to_string(),
            timestamp: SystemTime::UNIX_EPOCH,
        };
        let resp = DaemonResponse::Event { event };
        let mut encoded = serde_json::to_string(&resp).unwrap();
        encoded.push('\n');
        write_half.write_all(encoded.as_bytes()).await.unwrap();
        write_half.flush().await.unwrap();
        // Close connection — events loop breaks, exits 0.
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    minibox(&socket_path)
        .arg("events")
        .assert()
        .success()
        .stdout(predicate::str::contains("\"type\":\"created\""));
}
