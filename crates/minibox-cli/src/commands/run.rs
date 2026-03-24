//! `minibox run` — create and start a container.
//!
//! Unlike the other command modules, this module uses streaming responses
//! via [`DaemonClient`] to handle output in real time. It sets `ephemeral: true`
//! on the request so the daemon streams output back instead of returning
//! immediately with a container ID.
//!
//! # Streaming protocol
//!
//! After the `Run` request is sent the daemon emits a sequence of messages:
//!
//! * [`DaemonResponse::ContainerOutput`] — a base64-encoded chunk of bytes
//!   from the container's stdout or stderr; forwarded verbatim to the
//!   corresponding local stream.
//! * [`DaemonResponse::ContainerStopped`] — signals that the container has
//!   exited; the CLI exits with the container's exit code.
//! * [`DaemonResponse::ContainerCreated`] — only sent by older daemon builds
//!   that do not support streaming; the CLI prints the container ID and exits
//!   with code 0.
//! * [`DaemonResponse::Error`] — a fatal error from the daemon; printed to
//!   stderr and the CLI exits with code 1.

use anyhow::{Context, Result};
use base64::Engine;
use linuxbox::domain::NetworkMode;
use linuxbox::protocol::{DaemonRequest, DaemonResponse, OutputStreamKind};
use minibox_client::DaemonClient;
use std::io::Write;

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
    network: String,
) -> Result<()> {
    let network_mode = match network.as_str() {
        "none" => NetworkMode::None,
        "bridge" => NetworkMode::Bridge,
        "host" => NetworkMode::Host,
        "tailnet" => NetworkMode::Tailnet,
        other => {
            anyhow::bail!("unknown network mode: {other} (expected: none, bridge, host, tailnet)")
        }
    };

    let request = DaemonRequest::Run {
        image,
        tag: Some(tag),
        command,
        memory_limit_bytes,
        cpu_weight,
        ephemeral: true,
        network: Some(network_mode),
    };

    let client = DaemonClient::new().context("failed to create daemon client")?;
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    // Stream responses until ContainerStopped or an error.
    while let Some(response) = stream.next().await.context("stream error")? {
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
    use linuxbox::protocol::{DaemonResponse, OutputStreamKind};

    /// Verify that a base64-encoded stdout chunk round-trips correctly.
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

    /// Verify that a base64-encoded stderr chunk round-trips and retains the
    /// correct stream kind discriminant.
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
