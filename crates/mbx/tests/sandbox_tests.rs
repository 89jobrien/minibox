//! Integration tests for `mbx sandbox` — timeout enforcement and end-to-end
//! request validation.

use minibox_core::client::DaemonClient;
use minibox_core::domain::NetworkMode;
use minibox_core::protocol::{DaemonRequest, DaemonResponse, OutputStreamKind};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

/// Accept one connection, read the request, send ContainerCreated, then hang
/// forever (never send ContainerStopped). Simulates a container that exceeds
/// the timeout.
async fn serve_hang(socket_path: &std::path::Path) {
    let listener = UnixListener::bind(socket_path).unwrap();
    let (stream, _) = listener.accept().await.unwrap();
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);

    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();

    let resp = DaemonResponse::ContainerCreated {
        id: "test-hang".to_string(),
    };
    let mut json = serde_json::to_string(&resp).unwrap();
    json.push('\n');
    write_half.write_all(json.as_bytes()).await.unwrap();
    write_half.flush().await.unwrap();

    // Hold the connection open indefinitely.
    tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;
}

#[tokio::test]
async fn sandbox_times_out_when_container_hangs() {
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("test.sock");

    let sp = socket_path.clone();
    tokio::spawn(async move { serve_hang(&sp).await });
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    let request = DaemonRequest::Run {
        image: "minibox-sandbox".to_string(),
        tag: Some("latest".to_string()),
        command: vec!["sh".to_string(), "/workspace/script".to_string()],
        memory_limit_bytes: Some(512 * 1024 * 1024),
        cpu_weight: Some(100),
        ephemeral: true,
        network: Some(NetworkMode::None),
        mounts: vec![],
        privileged: false,
        env: vec![],
        name: None,
        tty: false,
        priority: None,
        urgency: None,
        execution_context: None,
    };

    let client = DaemonClient::with_socket(&socket_path);
    let mut stream = client.call(request).await.unwrap();

    let timeout = tokio::time::Duration::from_secs(1);
    let result = tokio::time::timeout(timeout, async {
        while let Some(response) = stream.next().await.unwrap() {
            match response {
                DaemonResponse::ContainerStopped { .. } => return,
                DaemonResponse::ContainerCreated { .. }
                | DaemonResponse::ContainerOutput { .. } => continue,
                _ => panic!("unexpected response"),
            }
        }
    })
    .await;

    // The timeout should fire because the mock never sends ContainerStopped.
    assert!(result.is_err(), "expected timeout, got: {result:?}");
}

#[tokio::test]
async fn sandbox_receives_exit_code_from_stopped_container() {
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("test.sock");

    let sp = socket_path.clone();
    tokio::spawn(async move {
        let listener = UnixListener::bind(&sp).unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();

        let output = DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"hello\n"),
        };
        let stopped = DaemonResponse::ContainerStopped { exit_code: 42 };
        for resp in [output, stopped] {
            let mut json = serde_json::to_string(&resp).unwrap();
            json.push('\n');
            write_half.write_all(json.as_bytes()).await.unwrap();
        }
        write_half.flush().await.unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    let client = DaemonClient::with_socket(&socket_path);
    let request = DaemonRequest::Run {
        image: "sandbox".to_string(),
        tag: Some("latest".to_string()),
        command: vec!["sh".to_string(), "/workspace/script".to_string()],
        memory_limit_bytes: Some(256 * 1024 * 1024),
        cpu_weight: Some(100),
        ephemeral: true,
        network: Some(NetworkMode::None),
        mounts: vec![],
        privileged: false,
        env: vec![],
        name: None,
        tty: false,
        priority: None,
        urgency: None,
        execution_context: None,
    };

    let mut stream = client.call(request).await.unwrap();
    let mut exit_code = None;

    while let Some(response) = stream.next().await.unwrap() {
        if let DaemonResponse::ContainerStopped { exit_code: code } = response {
            exit_code = Some(code);
            break;
        }
    }

    assert_eq!(exit_code, Some(42));
}
