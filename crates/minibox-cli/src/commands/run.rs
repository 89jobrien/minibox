//! `minibox run` — create and start a container.

use anyhow::{Context, Result};
use base64::Engine;
use minibox_lib::protocol::{DaemonRequest, DaemonResponse, OutputStreamKind};
use std::io::Write;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Execute the `run` subcommand.
///
/// Connects to the daemon, sends an ephemeral `DaemonRequest::Run`, then
/// streams `ContainerOutput` chunks to stdout/stderr until `ContainerStopped`
/// is received.  Exits with the container's exit code.
pub async fn execute(
    image: String,
    tag: String,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
) -> Result<()> {
    let request = DaemonRequest::Run {
        image,
        tag: Some(tag),
        command,
        memory_limit_bytes,
        cpu_weight,
        ephemeral: true,
    };

    // Connect to daemon socket.
    let path = super::socket_path();
    let stream = UnixStream::connect(&path)
        .await
        .with_context(|| format!("connecting to daemon at {path}"))?;

    let (read_half, mut writer) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    // Send request as a single JSON line.
    let mut payload = serde_json::to_string(&request).context("serialising request")?;
    payload.push('\n');
    writer
        .write_all(payload.as_bytes())
        .await
        .context("writing request")?;
    writer.flush().await.context("flushing request")?;

    // Stream responses until ContainerStopped or an error.
    while let Some(line) = lines.next_line().await.context("reading response")? {
        if line.is_empty() {
            continue;
        }
        let response: DaemonResponse =
            serde_json::from_str(&line).context("parsing daemon response")?;
        match response {
            DaemonResponse::ContainerOutput { stream, data } => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&data)
                    .context("failed to decode container output chunk")?;
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
                std::process::exit(exit_code);
            }
            DaemonResponse::ContainerCreated { id } => {
                // Old daemon — non-streaming path.
                println!("{id}");
                return Ok(());
            }
            DaemonResponse::Error { message } => {
                eprintln!("error: {message}");
                std::process::exit(1);
            }
            other => {
                eprintln!("run: unexpected response: {other:?}");
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_lib::protocol::{DaemonResponse, OutputStreamKind};

    #[test]
    fn decode_output_chunk() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"hello world\n");
        let response = DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data: encoded,
        };
        if let DaemonResponse::ContainerOutput { data, .. } = response {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&data)
                .unwrap();
            assert_eq!(decoded, b"hello world\n");
        } else {
            panic!("expected ContainerOutput");
        }
    }

    #[test]
    fn decode_stderr_chunk() {
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(b"error: something went wrong\n");
        let response = DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stderr,
            data: encoded,
        };
        if let DaemonResponse::ContainerOutput { stream, data } = response {
            assert_eq!(stream, OutputStreamKind::Stderr);
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&data)
                .unwrap();
            assert_eq!(decoded, b"error: something went wrong\n");
        } else {
            panic!("expected ContainerOutput");
        }
    }
}
