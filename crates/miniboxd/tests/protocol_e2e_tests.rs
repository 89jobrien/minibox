//! Cross-platform protocol e2e tests.
//!
//! These tests start a real miniboxd process and exercise the JSON-over-Unix-socket
//! protocol without requiring Linux namespaces, cgroups, or root. On macOS the daemon
//! dispatches to macbox; on Linux it uses the native adapter (but tests here avoid
//! operations that require root/cgroups).
//!
//! **Running:**
//! ```bash
//! cargo test -p miniboxd --test protocol_e2e_tests
//! ```

mod helpers;

use helpers::{find_binary, poll_until};
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Cross-platform daemon fixture (no cgroups, no root)
// ---------------------------------------------------------------------------

struct ProtocolFixture {
    child: Option<Child>,
    socket_path: std::path::PathBuf,
    _data_dir: TempDir,
    _run_dir: TempDir,
}

impl ProtocolFixture {
    fn start() -> Self {
        let data_dir = TempDir::with_prefix("minibox-proto-data-").expect("create temp data dir");
        let run_dir = TempDir::with_prefix("minibox-proto-run-").expect("create temp run dir");
        let socket_path = run_dir.path().join("miniboxd.sock");

        let daemon_bin = find_binary("miniboxd");

        let child = Command::new(&daemon_bin)
            .env("MINIBOX_DATA_DIR", data_dir.path())
            .env("MINIBOX_RUN_DIR", run_dir.path())
            .env("MINIBOX_SOCKET_PATH", &socket_path)
            .env("MINIBOX_METRICS_ADDR", "127.0.0.1:0")
            .env("RUST_LOG", "miniboxd=debug")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to start miniboxd at {daemon_bin:?}: {e}"));

        let sock = socket_path.clone();
        let started = poll_until(
            Duration::from_secs(10),
            Duration::from_millis(100),
            move || sock.exists(),
        );
        if !started {
            panic!("miniboxd did not create socket within 10s at {socket_path:?}");
        }

        Self {
            child: Some(child),
            socket_path,
            _data_dir: data_dir,
            _run_dir: run_dir,
        }
    }

    /// Send a request and collect all responses until a terminal one.
    fn request(&self, req: &DaemonRequest) -> Vec<DaemonResponse> {
        let mut stream = UnixStream::connect(&self.socket_path).expect("connect to daemon socket");
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .expect("set read timeout");

        let mut payload = serde_json::to_string(req).expect("serialize request");
        payload.push('\n');
        stream
            .write_all(payload.as_bytes())
            .expect("write request to socket");

        let reader = BufReader::new(&stream);
        let mut responses = Vec::new();

        for line in reader.lines() {
            let line = line.expect("read response line");
            if line.is_empty() {
                continue;
            }
            let resp: DaemonResponse = serde_json::from_str(&line).expect("deserialize response");
            let is_terminal = !matches!(&resp, DaemonResponse::ContainerOutput { .. });
            responses.push(resp);
            if is_terminal {
                break;
            }
        }

        responses
    }

    /// Send raw bytes and read the response line.
    fn raw_request(&self, payload: &[u8]) -> String {
        let mut stream = UnixStream::connect(&self.socket_path).expect("connect to daemon socket");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set read timeout");
        stream.write_all(payload).expect("write raw payload");

        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        reader.read_line(&mut line).expect("read response");
        line
    }
}

impl Drop for ProtocolFixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn protocol_list_empty_returns_container_list() {
    let fixture = ProtocolFixture::start();
    let responses = fixture.request(&DaemonRequest::List);

    assert_eq!(responses.len(), 1, "expected exactly one response");
    match &responses[0] {
        DaemonResponse::ContainerList { containers } => {
            assert!(
                containers.is_empty(),
                "fresh daemon should have no containers"
            );
        }
        other => panic!("expected ContainerList, got: {other:?}"),
    }
}

#[test]
fn protocol_stop_nonexistent_returns_error() {
    let fixture = ProtocolFixture::start();
    let responses = fixture.request(&DaemonRequest::Stop {
        id: "nonexistent-container-id".to_string(),
    });

    assert_eq!(responses.len(), 1);
    match &responses[0] {
        DaemonResponse::Error { message } => {
            assert!(
                message.to_lowercase().contains("not found")
                    || message.to_lowercase().contains("unknown")
                    || message.to_lowercase().contains("no container"),
                "error should indicate container not found, got: {message}"
            );
        }
        other => panic!("expected Error, got: {other:?}"),
    }
}

#[test]
fn protocol_remove_nonexistent_returns_error() {
    let fixture = ProtocolFixture::start();
    let responses = fixture.request(&DaemonRequest::Remove {
        id: "nonexistent-container-id".to_string(),
    });

    assert_eq!(responses.len(), 1);
    match &responses[0] {
        DaemonResponse::Error { message } => {
            assert!(
                message.to_lowercase().contains("not found")
                    || message.to_lowercase().contains("unknown")
                    || message.to_lowercase().contains("no container"),
                "error should indicate container not found, got: {message}"
            );
        }
        other => panic!("expected Error, got: {other:?}"),
    }
}

#[test]
fn protocol_malformed_json_returns_error() {
    let fixture = ProtocolFixture::start();
    let response = fixture.raw_request(b"this is not json\n");

    let resp: DaemonResponse =
        serde_json::from_str(response.trim()).expect("response should be valid JSON");
    match resp {
        DaemonResponse::Error { message } => {
            assert!(
                !message.is_empty(),
                "error message should not be empty for malformed input"
            );
        }
        other => panic!("expected Error for malformed JSON, got: {other:?}"),
    }
}

#[test]
fn protocol_empty_line_does_not_crash_daemon() {
    let fixture = ProtocolFixture::start();

    // Send empty line, then a valid request
    let mut stream = UnixStream::connect(&fixture.socket_path).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set timeout");
    stream.write_all(b"\n").expect("write empty line");

    // Send valid list request on same connection
    let req = serde_json::to_string(&DaemonRequest::List).unwrap() + "\n";
    stream
        .write_all(req.as_bytes())
        .expect("write list request");

    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read response");

    let resp: DaemonResponse =
        serde_json::from_str(line.trim()).expect("response should be valid JSON");
    assert!(
        matches!(resp, DaemonResponse::ContainerList { .. }),
        "daemon should still respond after empty line, got: {resp:?}"
    );
}

#[test]
fn protocol_multiple_requests_on_single_connection() {
    let fixture = ProtocolFixture::start();

    let mut stream = UnixStream::connect(&fixture.socket_path).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set timeout");

    // Send two list requests sequentially
    for _ in 0..2 {
        let req = serde_json::to_string(&DaemonRequest::List).unwrap() + "\n";
        stream.write_all(req.as_bytes()).expect("write request");
    }

    let mut reader = BufReader::new(&stream);
    for i in 0..2 {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read response");
        let resp: DaemonResponse = serde_json::from_str(line.trim())
            .unwrap_or_else(|e| panic!("response {i} not valid JSON: {e}, raw: {line}"));
        assert!(
            matches!(resp, DaemonResponse::ContainerList { .. }),
            "response {i} should be ContainerList, got: {resp:?}"
        );
    }
}

#[test]
fn protocol_pull_nonexistent_image_returns_error() {
    let fixture = ProtocolFixture::start();
    let responses = fixture.request(&DaemonRequest::Pull {
        image: "nonexistent-image-xyz-99999".to_string(),
        tag: None,
        platform: None,
    });

    // Should get an error (auth failure or not found)
    assert!(!responses.is_empty(), "should get at least one response");
    let last = responses.last().unwrap();
    match last {
        DaemonResponse::Error { .. } => {} // expected
        other => panic!("expected Error for nonexistent image pull, got: {other:?}"),
    }
}
