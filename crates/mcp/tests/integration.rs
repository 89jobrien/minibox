#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::unwrap_in_result
)]
//! Integration tests for the minibox MCP server.
//
// TODO(review): coverage gaps identified in review —
// - No test flips MINIBOX_MCP_ALLOW_* env vars and confirms the allow path actually
//   works (only safe_default()/deny path is unit-tested in policy.rs). from_env() is the
//   real binary's boot path and is completely untested.
// - No test calls minibox_stop/minibox_rm/minibox_pull through this real MCP/stdio
//   stack, so nothing proves a mutating tool is actually blocked (or allowed) end-to-end;
//   a regression dropping a validate_mutation() call would pass every existing test.
// - No test sends a privileged/bind-mount/host-network minibox_run through this stack to
//   confirm policy denial happens before the mock daemon ever receives a connection.
// - No test exercises daemon-unreachable, DaemonResponse::Error, malformed/truncated
//   frames, or output-limit overflow against the real client/server boundary.

use mcp::types::{PsOutput, RunContainerOutput};
use minibox_core::protocol::{ContainerInfo, DaemonRequest, DaemonResponse, OutputStreamKind};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParam;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use serde_json::json;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::process::Command;
use tokio::sync::oneshot;

fn mcp_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mcp"))
}

async fn spawn_client(socket_path: &Path) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let process = TokioChildProcess::new(Command::new(mcp_bin()).configure(|cmd| {
        cmd.env("MINIBOX_SOCKET_PATH", socket_path);
        cmd.env("RUST_LOG", "error");
    }))
    .expect("configure mcp child process");

    ().serve(process).await.expect("start mcp child process")
}

fn bind_mock(tmp: &TempDir) -> (UnixListener, PathBuf) {
    let socket_path = tmp.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind mock daemon socket");
    (listener, socket_path)
}

async fn mock_daemon_verify(
    listener: UnixListener,
    responses: Vec<DaemonResponse>,
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

    for response in responses {
        let mut encoded = serde_json::to_string(&response).expect("serialize response");
        encoded.push('\n');
        write_half
            .write_all(encoded.as_bytes())
            .await
            .expect("write response");
    }
    write_half.flush().await.expect("flush responses");
}

#[tokio::test]
async fn list_tools_exposes_minibox_tools() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("missing.sock");
    let service = spawn_client(&socket_path).await;

    let tools = service
        .list_tools(Option::default())
        .await
        .expect("list tools");
    let names: Vec<&str> = tools.tools.iter().map(|tool| tool.name.as_ref()).collect();

    assert!(names.contains(&"minibox_doctor"));
    assert!(names.contains(&"minibox_ps"));
    assert!(names.contains(&"minibox_run"));

    service.cancel().await.expect("cancel service");
}

#[tokio::test]
async fn minibox_ps_maps_to_list_request() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let (tx, rx) = oneshot::channel();
    tokio::spawn(mock_daemon_verify(
        listener,
        vec![DaemonResponse::ContainerList {
            containers: vec![ContainerInfo {
                id: "abc123".to_string(),
                name: Some("agent-test".to_string()),
                image: "alpine".to_string(),
                command: "/bin/true".to_string(),
                state: "stopped".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                pid: None,
            }],
        }],
        tx,
    ));

    let service = spawn_client(&socket_path).await;
    let result = service
        .call_tool(CallToolRequestParam {
            name: "minibox_ps".into(),
            arguments: Some(json!({}).as_object().cloned().unwrap()),
        })
        .await
        .expect("call minibox_ps");
    let output = result.into_typed::<PsOutput>().expect("typed ps output");
    let request = rx.await.expect("request captured");

    assert!(matches!(request, DaemonRequest::List));
    assert_eq!(output.containers.len(), 1);
    assert_eq!(output.containers[0].name.as_deref(), Some("agent-test"));

    service.cancel().await.expect("cancel service");
}

#[tokio::test]
async fn minibox_run_collects_streaming_output() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let (tx, rx) = oneshot::channel();
    tokio::spawn(mock_daemon_verify(
        listener,
        vec![
            DaemonResponse::ContainerCreated {
                id: "run123".to_string(),
            },
            DaemonResponse::ContainerOutput {
                stream: OutputStreamKind::Stdout,
                data: "aGVsbG8K".to_string(),
            },
            DaemonResponse::ContainerStopped { exit_code: 0 },
        ],
        tx,
    ));

    let service = spawn_client(&socket_path).await;
    let result = service
        .call_tool(CallToolRequestParam {
            name: "minibox_run".into(),
            arguments: Some(
                json!({
                    "image": "alpine",
                    "command": ["/bin/echo", "hello"]
                })
                .as_object()
                .cloned()
                .unwrap(),
            ),
        })
        .await
        .expect("call minibox_run");
    let output = result
        .into_typed::<RunContainerOutput>()
        .expect("typed run output");
    let request = rx.await.expect("request captured");

    match request {
        DaemonRequest::Run {
            image,
            ephemeral,
            auto_remove,
            ..
        } => {
            assert_eq!(image, "alpine");
            assert!(ephemeral);
            assert!(auto_remove);
        }
        other => panic!("expected Run request, got {other:?}"),
    }
    assert_eq!(output.container_id.as_deref(), Some("run123"));
    assert_eq!(output.stdout, "hello\n");
    assert_eq!(output.exit_code, Some(0));

    service.cancel().await.expect("cancel service");
}
