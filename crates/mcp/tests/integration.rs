#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::unwrap_in_result
)]
//! Integration tests for the minibox MCP server.
//
use mcp::types::{PsOutput, PullImageOutput, RunContainerOutput};
use minibox_core::protocol::{ContainerInfo, DaemonRequest, DaemonResponse, OutputStreamKind};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
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
    spawn_client_with_env(socket_path, &[]).await
}

async fn spawn_client_with_env(
    socket_path: &Path,
    env: &[(&str, &str)],
) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let process = TokioChildProcess::new(Command::new(mcp_bin()).configure(|cmd| {
        cmd.env("MINIBOX_SOCKET_PATH", socket_path);
        cmd.env("RUST_LOG", "error");
        for name in [
            "MINIBOX_MCP_ALLOW_MUTATION",
            "MINIBOX_MCP_ALLOW_BIND_MOUNTS",
            "MINIBOX_MCP_ALLOW_PRIVILEGED",
            "MINIBOX_MCP_ALLOW_HOST_NETWORK",
            "MINIBOX_MCP_MAX_OUTPUT_BYTES",
        ] {
            cmd.env_remove(name);
        }
        for (name, value) in env {
            cmd.env(name, value);
        }
    }))
    .expect("configure mcp child process");

    ().serve(process).await.expect("start mcp child process")
}

fn arguments(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value.as_object().cloned().expect("tool arguments object")
}

fn assert_error_code(error: &rmcp::service::ServiceError, expected: &str) {
    let rmcp::service::ServiceError::McpError(error) = error else {
        panic!("expected MCP error, got {error:?}");
    };
    let code = error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(serde_json::Value::as_str);
    assert_eq!(code, Some(expected));
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

async fn mock_daemon_raw(listener: UnixListener, frame: &[u8]) {
    let (stream, _) = listener.accept().await.expect("accept mock connection");
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .expect("read request line");
    write_half.write_all(frame).await.expect("write raw frame");
    write_half.flush().await.expect("flush raw frame");
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
async fn mutation_is_denied_through_stdio_before_daemon_contact() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("missing.sock");
    let service = spawn_client(&socket_path).await;

    let error = service
        .call_tool(
            CallToolRequestParams::new("minibox_pull")
                .with_arguments(arguments(json!({"image": "alpine"}))),
        )
        .await
        .expect_err("default policy must deny pull");

    assert_error_code(&error, "minibox::mcp::policy_denied");
    service.cancel().await.expect("cancel service");
}

#[tokio::test]
async fn mutation_is_allowed_through_stdio_when_enabled() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let (tx, rx) = oneshot::channel();
    tokio::spawn(mock_daemon_verify(
        listener,
        vec![DaemonResponse::Success {
            message: "pulled alpine".to_string(),
        }],
        tx,
    ));
    let service =
        spawn_client_with_env(&socket_path, &[("MINIBOX_MCP_ALLOW_MUTATION", "true")]).await;

    let result = service
        .call_tool(
            CallToolRequestParams::new("minibox_pull")
                .with_arguments(arguments(json!({"image": "alpine"}))),
        )
        .await
        .expect("enabled mutation should reach daemon");
    let output = result
        .into_typed::<PullImageOutput>()
        .expect("typed pull output");

    assert!(matches!(
        rx.await.expect("request captured"),
        DaemonRequest::Pull { .. }
    ));
    assert_eq!(output.message, "pulled alpine");
    service.cancel().await.expect("cancel service");
}

#[tokio::test]
async fn risky_run_options_are_denied_through_stdio() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("missing.sock");
    let cases = [
        json!({"image": "alpine", "privileged": true}),
        json!({
            "image": "alpine",
            "mounts": [{"host_path": "/tmp", "container_path": "/host"}]
        }),
        json!({"image": "alpine", "network": "host"}),
    ];

    for input in cases {
        let service = spawn_client(&socket_path).await;
        let error = service
            .call_tool(CallToolRequestParams::new("minibox_run").with_arguments(arguments(input)))
            .await
            .expect_err("risky run must be denied");
        assert_error_code(&error, "minibox::mcp::policy_denied");
        service.cancel().await.expect("cancel service");
    }
}

#[tokio::test]
async fn daemon_error_is_reported_through_stdio() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let (tx, _rx) = oneshot::channel();
    tokio::spawn(mock_daemon_verify(
        listener,
        vec![DaemonResponse::Error {
            message: "daemon rejected request".to_string(),
        }],
        tx,
    ));
    let service = spawn_client(&socket_path).await;

    let error = service
        .call_tool(CallToolRequestParams::new("minibox_ps").with_arguments(arguments(json!({}))))
        .await
        .expect_err("daemon error must cross stdio boundary");

    assert_error_code(&error, "minibox::mcp::daemon_error");
    service.cancel().await.expect("cancel service");
}

#[tokio::test]
async fn unreachable_daemon_is_reported_through_stdio() {
    let tmp = TempDir::new().expect("tempdir");
    let socket_path = tmp.path().join("missing.sock");
    let service = spawn_client(&socket_path).await;

    let error = service
        .call_tool(CallToolRequestParams::new("minibox_ps").with_arguments(arguments(json!({}))))
        .await
        .expect_err("unreachable daemon must cross stdio boundary");

    assert_error_code(&error, "minibox::mcp::daemon_connection");
    service.cancel().await.expect("cancel service");
}

#[tokio::test]
async fn malformed_daemon_frame_is_reported_through_stdio() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    tokio::spawn(mock_daemon_raw(listener, b"not-json\n"));
    let service = spawn_client(&socket_path).await;

    let error = service
        .call_tool(CallToolRequestParams::new("minibox_ps").with_arguments(arguments(json!({}))))
        .await
        .expect_err("malformed frame must cross stdio boundary");

    assert_error_code(&error, "minibox::mcp::protocol_error");
    service.cancel().await.expect("cancel service");
}

#[tokio::test]
async fn output_overflow_is_gracefully_truncated_through_stdio() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let (tx, _rx) = oneshot::channel();
    tokio::spawn(mock_daemon_verify(
        listener,
        vec![
            DaemonResponse::ContainerOutput {
                stream: OutputStreamKind::Stdout,
                data: "YWFhYWFh".to_string(),
            },
            DaemonResponse::ContainerOutput {
                stream: OutputStreamKind::Stderr,
                data: "YmJiYmJi".to_string(),
            },
            DaemonResponse::ContainerStopped { exit_code: 0 },
        ],
        tx,
    ));
    let service =
        spawn_client_with_env(&socket_path, &[("MINIBOX_MCP_MAX_OUTPUT_BYTES", "8")]).await;

    let result = service
        .call_tool(
            CallToolRequestParams::new("minibox_run")
                .with_arguments(arguments(json!({"image": "alpine"}))),
        )
        .await
        .expect("oversized run output should be normalized");
    let output = result
        .into_typed::<RunContainerOutput>()
        .expect("typed run output");

    assert_eq!(output.stdout, "aaaaaa");
    assert_eq!(output.stderr, "bb");
    assert_eq!(output.exit_code, Some(0));
    assert!(output.truncated);
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
        .call_tool(
            CallToolRequestParams::new("minibox_ps")
                .with_arguments(json!({}).as_object().cloned().unwrap()),
        )
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
        .call_tool(
            CallToolRequestParams::new("minibox_run").with_arguments(
                json!({"image": "alpine", "command": ["/bin/echo", "hello"]})
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
        )
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
