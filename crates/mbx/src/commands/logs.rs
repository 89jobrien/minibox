//! `minibox logs` — stream container log output.
//!
//! Sends a `DaemonRequest::ContainerLogs` to the daemon, then processes the
//! response stream:
//!
//! * [`DaemonResponse::LogLine`] — print to stdout or stderr depending on the
//!   originating stream.
//! * [`DaemonResponse::Success`] — terminal; all logs delivered, exit 0.
//! * [`DaemonResponse::Error`] — terminal error; print to stderr, exit 1.

use anyhow::{Context as _, Result};
use minibox_client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse, OutputStreamKind};

/// Execute the `logs` subcommand.
///
/// Connects to the daemon, sends a `ContainerLogs` request, then prints each
/// received log line to the appropriate output stream.  Exits when a terminal
/// response (`Success` or `Error`) is received, or when the connection closes.
pub async fn execute(
    container_id: String,
    follow: bool,
    socket_path: &std::path::Path,
) -> Result<()> {
    let request = DaemonRequest::ContainerLogs {
        container_id,
        follow,
    };

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    while let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::LogLine { stream, line } => match stream {
                OutputStreamKind::Stdout => println!("{line}"),
                OutputStreamKind::Stderr => eprintln!("{line}"),
            },
            DaemonResponse::Success { .. } => {
                return Ok(());
            }
            DaemonResponse::Error { message } => {
                eprintln!("error: {message}");
                std::process::exit(1);
            }
            other => {
                eprintln!("logs: unexpected response: {other:?}");
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::protocol::DaemonResponse;

    /// Verify the `ContainerLogs` request serialises with the correct type tag.
    #[test]
    fn logs_request_has_type_tag() {
        let req = DaemonRequest::ContainerLogs {
            container_id: "ctr1".to_string(),
            follow: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(
            json.contains("\"type\":\"ContainerLogs\""),
            "serialised ContainerLogs request missing type tag: {json}"
        );
    }

    /// Verify `follow: true` serialises correctly.
    #[test]
    fn logs_request_follow_field() {
        let req = DaemonRequest::ContainerLogs {
            container_id: "ctr2".to_string(),
            follow: true,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(
            json.contains("\"follow\":true"),
            "follow field missing or false: {json}"
        );
    }

    /// Verify that a `LogLine` response deserialises correctly.
    #[test]
    fn log_line_response_deserialises() {
        let json = r#"{"type":"LogLine","stream":"stdout","line":"hello world"}"#;
        let resp: DaemonResponse = serde_json::from_str(json).unwrap();
        match resp {
            DaemonResponse::LogLine { stream, line } => {
                assert!(matches!(stream, OutputStreamKind::Stdout));
                assert_eq!(line, "hello world");
            }
            _ => panic!("expected LogLine, got {resp:?}"),
        }
    }

    /// Verify `execute` returns Ok when daemon sends `Success` after log lines.
    #[cfg(unix)]
    #[tokio::test]
    async fn execute_returns_ok_on_success() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixListener;

        let tmp = tempfile::TempDir::new().unwrap();
        let socket_path = tmp.path().join("logs_test.sock");
        let sp = socket_path.clone();

        tokio::spawn(async move {
            let listener = UnixListener::bind(&sp).unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let (read_half, mut write_half) = tokio::io::split(stream);
            let mut reader = BufReader::new(read_half);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();

            let responses = [
                DaemonResponse::LogLine {
                    stream: OutputStreamKind::Stdout,
                    line: "line one".to_string(),
                },
                DaemonResponse::LogLine {
                    stream: OutputStreamKind::Stderr,
                    line: "line two".to_string(),
                },
                DaemonResponse::Success {
                    message: "end of log".to_string(),
                },
            ];
            for resp in responses {
                let mut encoded = serde_json::to_string(&resp).unwrap();
                encoded.push('\n');
                write_half.write_all(encoded.as_bytes()).await.unwrap();
            }
            write_half.flush().await.unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let result = execute("abc123".to_string(), false, &socket_path).await;
        assert!(result.is_ok(), "execute should return Ok: {result:?}");
    }

    /// Verify the CLI parser accepts `logs <id>` and `logs <id> --follow`.
    #[test]
    fn cli_parses_logs_subcommand() {
        use crate::main_tests_shim::parse_logs;
        let (id, follow) = parse_logs(&["minibox", "logs", "abc123"]);
        assert_eq!(id, "abc123");
        assert!(!follow);
    }

    #[test]
    fn cli_parses_logs_follow_flag() {
        use crate::main_tests_shim::parse_logs;
        let (id, follow) = parse_logs(&["minibox", "logs", "abc123", "--follow"]);
        assert_eq!(id, "abc123");
        assert!(follow);
    }
}
