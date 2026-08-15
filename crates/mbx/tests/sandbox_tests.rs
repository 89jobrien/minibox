#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::redundant_field_names,
    clippy::uninlined_format_args,
    clippy::redundant_clone,
    clippy::redundant_closure,
    clippy::single_char_pattern,
    clippy::unwrap_in_result,
    clippy::collapsible_if,
    clippy::match_same_arms,
    clippy::only_used_in_recursion,
    clippy::used_underscore_binding,
    clippy::map_unwrap_or,
    clippy::manual_assert,
    clippy::as_ptr_cast_mut,
    clippy::ptr_as_ptr,
    clippy::must_use_candidate,
    clippy::used_underscore_items,
    clippy::missing_const_for_fn,
    clippy::manual_string_new,
    clippy::semicolon_if_nothing_returned,
    clippy::unreadable_literal,
    clippy::default_constructed_unit_structs,
    clippy::ref_as_ptr,
    clippy::allow_attributes_without_reason,
    clippy::redundant_closure_for_method_calls,
    clippy::needless_raw_string_hashes,
    clippy::manual_is_variant_and,
    clippy::ignore_without_reason,
    clippy::default_trait_access,
    clippy::cast_lossless,
    clippy::match_wild_err_arm,
    clippy::format_push_string,
    clippy::bool_assert_comparison,
    clippy::struct_excessive_bools,
    clippy::duration_suboptimal_units,
    clippy::unnecessary_map_or
)]
//! Integration tests for `mbx sandbox` — timeout enforcement and end-to-end
//! request validation.

use minibox_core::client::DaemonClient;
use minibox_core::domain::NetworkMode;
use minibox_core::protocol::{DaemonResponse, OutputStreamKind};
use minibox_macros::test_run;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

mod test_helpers;
use test_helpers::wait_for_socket;

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
    wait_for_socket(&socket_path, 2000).await;

    let request = test_run!(
        image: "minibox-sandbox".to_string(),
        tag: Some("latest".to_string()),
        command: vec!["sh".to_string(), "/workspace/script".to_string()],
        memory_limit_bytes: Some(512 * 1024 * 1024),
        cpu_weight: Some(100),
        ephemeral: true,
        network: Some(NetworkMode::None),
    );

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
    wait_for_socket(&socket_path, 2000).await;

    let client = DaemonClient::with_socket(&socket_path);
    let request = test_run!(
        image: "sandbox".to_string(),
        tag: Some("latest".to_string()),
        command: vec!["sh".to_string(), "/workspace/script".to_string()],
        memory_limit_bytes: Some(256 * 1024 * 1024),
        cpu_weight: Some(100),
        ephemeral: true,
        network: Some(NetworkMode::None),
    );

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
