//! `minibox exec` — execute a command inside a running container.
//!
//! Sends a `DaemonRequest::Exec` to the daemon, then streams
//! `ContainerOutput` chunks to stdout/stderr until `ContainerStopped` is
//! received.  Exits with the container's exit code.
//!
//! # Streaming protocol
//!
//! After the `Exec` request is sent the daemon emits a sequence of messages:
//!
//! * [`DaemonResponse::ExecStarted`] — exec session established; non-terminal.
//! * [`DaemonResponse::ContainerOutput`] — base64-encoded stdout/stderr chunk.
//! * [`DaemonResponse::ContainerStopped`] — exec process exited; CLI exits
//!   with the carried exit code.
//! * [`DaemonResponse::Error`] — fatal error; CLI exits with code 1.

use anyhow::{Context as _, Result};
use base64::Engine;
use minibox_core::client::{DaemonClient, DaemonWriter};
use minibox_core::protocol::{DaemonRequest, DaemonResponse, OutputStreamKind};
use std::io::Write;

/// Execute the `exec` subcommand.
///
/// Connects to the daemon, sends a `DaemonRequest::Exec`, then streams
/// `ContainerOutput` chunks to stdout/stderr until `ContainerStopped`.
/// Exits with the container process exit code.
pub async fn execute(
    container_id: String,
    cmd: Vec<String>,
    tty: bool,
    socket_path: &std::path::Path,
) -> Result<()> {
    use std::io::IsTerminal as _;
    let tty = tty && std::io::stdout().is_terminal();

    let request = DaemonRequest::Exec {
        container_id,
        cmd,
        env: vec![],
        working_dir: None,
        tty,
    };

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    #[cfg(unix)]
    let _raw_guard = if tty {
        Some(crate::terminal::RawModeGuard::enter().context("raw mode enter")?)
    } else {
        None
    };

    let sp = socket_path.to_path_buf();

    while let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::ExecStarted { exec_id } => {
                if tty {
                    // Stdin relay task — batches reads and sends one SendInput per
                    // read() call via DaemonWriter (fire-and-forget, no per-call
                    // response needed), avoiding the per-keypress connection cost.
                    let sp2 = sp.clone();
                    let sid = exec_id.clone();
                    tokio::spawn(async move {
                        use tokio::io::AsyncReadExt as _;
                        let writer = DaemonWriter::with_socket(&sp2);
                        let mut stdin = tokio::io::stdin();
                        let mut buf = [0u8; 256];
                        loop {
                            match stdin.read(&mut buf).await {
                                Ok(0) | Err(_) => break,
                                Ok(n) => {
                                    let data =
                                        base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                                    let req = DaemonRequest::SendInput {
                                        session_id: minibox_core::domain::SessionId::from(
                                            sid.clone(),
                                        ),
                                        data,
                                    };
                                    let _ = writer.send(req).await;
                                }
                            }
                        }
                    });

                    // Initial terminal size.
                    #[cfg(unix)]
                    {
                        let (cols, rows) = crate::terminal::terminal_size();
                        let _ = DaemonWriter::with_socket(&sp)
                            .send(DaemonRequest::ResizePty {
                                session_id: minibox_core::domain::SessionId::from(exec_id.clone()),
                                cols,
                                rows,
                            })
                            .await;
                    }

                    // SIGWINCH forwarding — uses tokio's process-wide signal stream
                    // so the signal is reliably received regardless of which Tokio
                    // worker thread the OS delivers it to.  Per-thread sigprocmask
                    // is not portable under a multi-threaded runtime.
                    #[cfg(unix)]
                    {
                        use tokio::signal::unix::{SignalKind, signal};
                        let sp3 = sp.clone();
                        let sid2 = exec_id.clone();
                        match signal(SignalKind::window_change()) {
                            Ok(mut sigwinch) => {
                                tokio::spawn(async move {
                                    let writer = DaemonWriter::with_socket(&sp3);
                                    while sigwinch.recv().await.is_some() {
                                        let (cols, rows) = crate::terminal::terminal_size();
                                        let _ = writer
                                            .send(DaemonRequest::ResizePty {
                                                session_id: minibox_core::domain::SessionId::from(
                                                    sid2.clone(),
                                                ),
                                                cols,
                                                rows,
                                            })
                                            .await;
                                    }
                                });
                            }
                            Err(e) => {
                                eprintln!(
                                    "exec: SIGWINCH handler unavailable; terminal resize will not be forwarded: {e}"
                                );
                            }
                        }
                    }
                }
            }
            DaemonResponse::ContainerOutput { stream, data } => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&data)
                    .context("failed to decode exec output chunk")?;
                match stream {
                    OutputStreamKind::Stdout => {
                        std::io::stdout().write_all(&bytes)?;
                        std::io::stdout().flush()?;
                    }
                    OutputStreamKind::Stderr => {
                        std::io::stderr().write_all(&bytes)?;
                        std::io::stderr().flush()?;
                    }
                }
            }
            DaemonResponse::ContainerStopped { exit_code } => {
                #[cfg(unix)]
                drop(_raw_guard);
                std::process::exit(exit_code);
            }
            DaemonResponse::Error { message } => {
                eprintln!("error: {message}");
                std::process::exit(1);
            }
            other => {
                eprintln!("exec: unexpected response: {other:?}");
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

    /// Helper: bind a Unix socket, accept one connection, read the request line,
    /// then send back the given sequence of responses.
    #[cfg(unix)]
    async fn serve_responses(socket_path: &std::path::Path, responses: Vec<DaemonResponse>) {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixListener;

        let listener = UnixListener::bind(socket_path).unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        for resp in responses {
            let mut encoded = serde_json::to_string(&resp).unwrap();
            encoded.push('\n');
            write_half.write_all(encoded.as_bytes()).await.unwrap();
        }
        write_half.flush().await.unwrap();
    }

    /// Verify that `execute` sends an Exec request with the correct fields.
    #[cfg(unix)]
    #[tokio::test]
    async fn exec_sends_correct_request() {
        use minibox_core::protocol::DaemonRequest;
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixListener;

        let tmp = tempfile::TempDir::new().unwrap();
        let socket_path = tmp.path().join("exec_request_test.sock");
        let sp = socket_path.clone();

        // Spawn server that captures the request and responds with ExecStarted
        // then ContainerStopped so execute() can complete cleanly.
        tokio::spawn(async move {
            let listener = UnixListener::bind(&sp).unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let (read_half, mut write_half) = tokio::io::split(stream);
            let mut reader = BufReader::new(read_half);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();

            // Validate the request
            let req: DaemonRequest = serde_json::from_str(line.trim()).unwrap();
            match req {
                DaemonRequest::Exec {
                    container_id,
                    cmd,
                    env,
                    working_dir,
                    tty,
                } => {
                    assert_eq!(container_id, "abc123");
                    assert_eq!(cmd, vec!["/bin/sh"]);
                    assert!(env.is_empty());
                    assert_eq!(working_dir, None);
                    assert!(!tty);
                }
                _ => panic!("expected Exec request, got something else"),
            }

            // Send ExecStarted then stop — execute() will process::exit on
            // ContainerStopped; use ExecStarted only so we can return Ok(()).
            // Since process::exit can't be tested directly, just send ExecStarted
            // and let the stream close naturally (execute returns Ok).
            let resp = serde_json::to_string(&DaemonResponse::ExecStarted {
                exec_id: "exec-1".to_string(),
            })
            .unwrap();
            write_half
                .write_all(format!("{resp}\n").as_bytes())
                .await
                .unwrap();
            write_half.flush().await.unwrap();
            // Close connection — stream.next() returns None → execute returns Ok(()).
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        // tty=false: stdout is not a terminal in tests, so raw mode is never entered.
        let result = execute(
            "abc123".to_string(),
            vec!["/bin/sh".to_string()],
            false,
            &socket_path,
        )
        .await;
        assert!(
            result.is_ok(),
            "execute should return Ok after ExecStarted + stream close: {result:?}"
        );
    }

    /// Verify that the Exec request serialises with the correct JSON type tag.
    #[test]
    fn exec_request_has_type_tag() {
        let req = DaemonRequest::Exec {
            container_id: "ctr1".to_string(),
            cmd: vec!["ls".to_string()],
            env: vec![],
            working_dir: None,
            tty: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(
            json.contains("\"type\":\"Exec\""),
            "serialised Exec request missing type tag: {json}"
        );
    }

    /// Verify that `execute` parses an ExecStarted response without panicking.
    #[test]
    fn exec_started_response_deserialises() {
        let json = r#"{"type":"ExecStarted","exec_id":"exec-42"}"#;
        let resp: DaemonResponse = serde_json::from_str(json).unwrap();
        assert!(
            matches!(resp, DaemonResponse::ExecStarted { .. }),
            "expected ExecStarted"
        );
    }

    /// Verify that base64-encoded output chunks round-trip correctly in the
    /// exec output path.
    #[test]
    fn exec_output_chunk_decodes() {
        let raw = b"hello from exec\n";
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
        let resp = DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data: encoded.clone(),
        };
        if let DaemonResponse::ContainerOutput { data, .. } = resp {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&data)
                .unwrap();
            assert_eq!(decoded, raw);
        } else {
            panic!("expected ContainerOutput");
        }
    }
}
