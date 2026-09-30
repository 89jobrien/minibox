#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::unwrap_in_result
)]
//! Integration tests for the minibox MCP server.

use mcp::types::{PsOutput, RunContainerOutput};
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
        cmd.envs(env.iter().copied());
    }))
    .expect("configure mcp child process");

    ().serve(process).await.expect("start mcp child process")
}

async fn forward_run_with_opt_in(env_name: &str, arguments: serde_json::Value) -> DaemonRequest {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let (tx, rx) = oneshot::channel();
    tokio::spawn(mock_daemon_verify(
        listener,
        vec![DaemonResponse::ContainerStopped { exit_code: 0 }],
        tx,
    ));
    let service = spawn_client_with_env(&socket_path, &[(env_name, "true")]).await;

    service
        .call_tool(
            CallToolRequestParams::new("minibox_run")
                .with_arguments(arguments.as_object().cloned().expect("object arguments")),
        )
        .await
        .expect("opted-in unsafe run should succeed");
    let request = rx.await.expect("request captured");
    service.cancel().await.expect("cancel service");
    request
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

#[tokio::test]
async fn mutation_tools_reach_daemon_when_opted_in() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let (tx, rx) = oneshot::channel();
    tokio::spawn(mock_daemon_verify(
        listener,
        vec![DaemonResponse::ContainerStopped { exit_code: 0 }],
        tx,
    ));

    let service =
        spawn_client_with_env(&socket_path, &[("MINIBOX_MCP_ALLOW_MUTATION", "true")]).await;
    let result = service
        .call_tool(
            CallToolRequestParams::new("minibox_stop")
                .with_arguments(json!({"id": "abc123"}).as_object().cloned().unwrap()),
        )
        .await
        .expect("opted-in mutation should succeed");
    let request = rx.await.expect("request captured");

    assert!(matches!(request, DaemonRequest::Stop { id } if id == "abc123"));
    assert!(result.into_typed::<mcp::types::SimpleOutput>().is_ok());

    service.cancel().await.expect("cancel service");
}

fn assert_error_code(error: rmcp::service::ServiceError, expected: &str) {
    let rmcp::service::ServiceError::McpError(error) = error else {
        panic!("expected structured MCP error, got {error:?}");
    };
    assert_eq!(
        error
            .data
            .as_ref()
            .and_then(|data| data.get("code"))
            .and_then(serde_json::Value::as_str),
        Some(expected)
    );
}

#[tokio::test]
async fn mutation_tools_are_denied_before_daemon_connect() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let service = spawn_client(&socket_path).await;

    for (tool, arguments) in [
        ("minibox_stop", json!({"id": "abc123"})),
        ("minibox_rm", json!({"id": "abc123"})),
        ("minibox_pull", json!({"image": "alpine"})),
    ] {
        let error = service
            .call_tool(
                CallToolRequestParams::new(tool)
                    .with_arguments(arguments.as_object().cloned().unwrap()),
            )
            .await
            .expect_err("mutation must be denied by default");
        assert_error_code(error, "minibox::mcp::policy_denied");
    }

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err(),
        "denied mutations must not connect to the daemon"
    );
    service.cancel().await.expect("cancel service");
}

#[tokio::test]
async fn unsafe_runs_are_denied_before_daemon_connect() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let service = spawn_client(&socket_path).await;

    for arguments in [
        json!({"image": "alpine", "privileged": true}),
        json!({
            "image": "alpine",
            "mounts": [{"host_path": "/tmp", "container_path": "/host"}]
        }),
        json!({"image": "alpine", "network": "host"}),
    ] {
        let error = service
            .call_tool(
                CallToolRequestParams::new("minibox_run")
                    .with_arguments(arguments.as_object().cloned().unwrap()),
            )
            .await
            .expect_err("unsafe run must be denied by default");
        assert_error_code(error, "minibox::mcp::policy_denied");
    }

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err(),
        "denied runs must not connect to the daemon"
    );
    service.cancel().await.expect("cancel service");
}

#[tokio::test]
async fn privileged_run_reaches_daemon_when_opted_in() {
    let request = forward_run_with_opt_in(
        "MINIBOX_MCP_ALLOW_PRIVILEGED",
        json!({"image": "alpine", "privileged": true}),
    )
    .await;

    assert!(matches!(
        request,
        DaemonRequest::Run {
            privileged: true,
            ..
        }
    ));
}

#[tokio::test]
async fn bind_mount_run_reaches_daemon_when_opted_in() {
    let request = forward_run_with_opt_in(
        "MINIBOX_MCP_ALLOW_BIND_MOUNTS",
        json!({
            "image": "alpine",
            "mounts": [{"host_path": "/tmp", "container_path": "/host", "read_only": true}]
        }),
    )
    .await;

    let DaemonRequest::Run { mounts, .. } = request else {
        panic!("expected Run request");
    };
    assert_eq!(mounts.len(), 1);
    assert_eq!(mounts[0].host_path, PathBuf::from("/tmp"));
    assert_eq!(mounts[0].container_path, PathBuf::from("/host"));
    assert!(mounts[0].read_only);
}

#[tokio::test]
async fn host_network_run_reaches_daemon_when_opted_in() {
    let request = forward_run_with_opt_in(
        "MINIBOX_MCP_ALLOW_HOST_NETWORK",
        json!({"image": "alpine", "network": "host"}),
    )
    .await;

    assert!(matches!(
        request,
        DaemonRequest::Run {
            network: Some(minibox_core::domain::NetworkMode::Host),
            ..
        }
    ));
}

#[tokio::test]
async fn rm_reaches_daemon_when_opted_in() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let (tx, rx) = oneshot::channel();
    tokio::spawn(mock_daemon_verify(
        listener,
        vec![DaemonResponse::Success {
            message: "removed".to_string(),
        }],
        tx,
    ));
    let service =
        spawn_client_with_env(&socket_path, &[("MINIBOX_MCP_ALLOW_MUTATION", "true")]).await;

    service
        .call_tool(
            CallToolRequestParams::new("minibox_rm")
                .with_arguments(json!({"id": "abc123"}).as_object().cloned().unwrap()),
        )
        .await
        .expect("opted-in rm should succeed");
    assert!(matches!(
        rx.await.expect("request captured"),
        DaemonRequest::Remove { id } if id == "abc123"
    ));
    service.cancel().await.expect("cancel service");
}

#[tokio::test]
async fn pull_reaches_daemon_when_opted_in() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let (tx, rx) = oneshot::channel();
    tokio::spawn(mock_daemon_verify(
        listener,
        vec![DaemonResponse::Success {
            message: "pulled".to_string(),
        }],
        tx,
    ));
    let service =
        spawn_client_with_env(&socket_path, &[("MINIBOX_MCP_ALLOW_MUTATION", "true")]).await;

    service
        .call_tool(
            CallToolRequestParams::new("minibox_pull")
                .with_arguments(json!({"image": "alpine"}).as_object().cloned().unwrap()),
        )
        .await
        .expect("opted-in pull should succeed");
    assert!(matches!(
        rx.await.expect("request captured"),
        DaemonRequest::Pull { image, .. } if image == "alpine"
    ));
    service.cancel().await.expect("cancel service");
}

#[tokio::test]
async fn client_server_boundary_preserves_error_kinds() {
    let missing = TempDir::new().expect("tempdir");
    let service = spawn_client(&missing.path().join("missing.sock")).await;
    let error = service
        .call_tool(
            CallToolRequestParams::new("minibox_ps")
                .with_arguments(json!({}).as_object().cloned().unwrap()),
        )
        .await
        .expect_err("missing daemon must fail");
    assert_error_code(error, "minibox::mcp::daemon_connection");
    service.cancel().await.expect("cancel service");

    for (response, expected) in [
        (
            "{\"type\":\"Error\",\"message\":\"daemon failed\"}\n",
            "minibox::mcp::daemon_error",
        ),
        ("not-json\n", "minibox::mcp::protocol_error"),
    ] {
        let tmp = TempDir::new().expect("tempdir");
        let (listener, socket_path) = bind_mock(&tmp);
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept mock connection");
            let mut reader = BufReader::new(&mut stream);
            let mut request = String::new();
            reader.read_line(&mut request).await.expect("read request");
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        });
        let service = spawn_client(&socket_path).await;
        let error = service
            .call_tool(
                CallToolRequestParams::new("minibox_ps")
                    .with_arguments(json!({}).as_object().cloned().unwrap()),
            )
            .await
            .expect_err("daemon boundary error must be reported");
        assert_error_code(error, expected);
        service.cancel().await.expect("cancel service");
    }
}

#[tokio::test]
async fn minibox_run_rejects_metadata_overflow() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let (tx, _rx) = oneshot::channel();
    tokio::spawn(mock_daemon_verify(
        listener,
        vec![DaemonResponse::ContainerCreated {
            id: "x".repeat(64 * 1024),
        }],
        tx,
    ));
    let service = spawn_client(&socket_path).await;

    let error = service
        .call_tool(
            CallToolRequestParams::new("minibox_run")
                .with_arguments(json!({"image": "alpine"}).as_object().cloned().unwrap()),
        )
        .await
        .expect_err("oversized run metadata must be rejected");
    assert_error_code(error, "minibox::mcp::output_limit_exceeded");
    service.cancel().await.expect("cancel service");
}

#[tokio::test]
async fn minibox_run_reports_malformed_base64_as_protocol_error() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let (tx, _rx) = oneshot::channel();
    tokio::spawn(mock_daemon_verify(
        listener,
        vec![DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data: "not-base64!".to_string(),
        }],
        tx,
    ));
    let service = spawn_client(&socket_path).await;

    let error = service
        .call_tool(
            CallToolRequestParams::new("minibox_run")
                .with_arguments(json!({"image": "alpine"}).as_object().cloned().unwrap()),
        )
        .await
        .expect_err("malformed output must be rejected");
    assert_error_code(error, "minibox::mcp::protocol_error");
    service.cancel().await.expect("cancel service");
}

#[tokio::test]
async fn minibox_run_returns_bounded_truncated_output() {
    let tmp = TempDir::new().expect("tempdir");
    let (listener, socket_path) = bind_mock(&tmp);
    let (tx, _rx) = oneshot::channel();
    tokio::spawn(mock_daemon_verify(
        listener,
        vec![
            DaemonResponse::ContainerCreated {
                id: "run123".to_string(),
            },
            DaemonResponse::ContainerOutput {
                stream: OutputStreamKind::Stdout,
                data: "aGVsbG8=".to_string(),
            },
            DaemonResponse::ContainerOutput {
                stream: OutputStreamKind::Stderr,
                data: "d29ybGQ=".to_string(),
            },
            DaemonResponse::ContainerStopped { exit_code: 0 },
        ],
        tx,
    ));
    let service =
        spawn_client_with_env(&socket_path, &[("MINIBOX_MCP_MAX_OUTPUT_BYTES", "5")]).await;

    let result = service
        .call_tool(
            CallToolRequestParams::new("minibox_run")
                .with_arguments(json!({"image": "alpine"}).as_object().cloned().unwrap()),
        )
        .await
        .expect("large run output should be truncated, not rejected");
    let output = result
        .into_typed::<RunContainerOutput>()
        .expect("typed run output");

    assert_eq!(output.stdout.len() + output.stderr.len(), 5);
    assert_eq!(output.stdout, "hello");
    assert!(output.stderr.is_empty());
    assert!(output.truncated);
    assert_eq!(output.exit_code, Some(0));

    service.cancel().await.expect("cancel service");
}
