//! VzProxy — JSON-over-vsock request/response handler.
//!
//! Forwards [`DaemonRequest`] messages to an in-VM miniboxd agent over a vsock
//! connection and collects [`DaemonResponse`] messages until a terminal response
//! is received. Handles streaming responses (`ContainerOutput`) transparently.

use anyhow::{Context, Result};
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Determines if a response is terminal (stops the response stream).
///
/// Terminal responses depend on whether the request is ephemeral:
///
/// **For any request**:
/// - `ContainerStopped` (always terminal)
/// - `Error` (always terminal)
/// - `Success` (always terminal)
/// - `ContainerList` (always terminal)
///
/// **For non-ephemeral `Run` requests**:
/// - `ContainerCreated` is terminal (daemon sends it and closes connection)
///
/// **For ephemeral `Run` requests**:
/// - `ContainerCreated` is non-terminal (followed by `ContainerOutput` chunks)
/// - `ContainerOutput` is non-terminal (multiple chunks can follow)
///
/// # Proxy vs Server Design
///
/// This differs from `daemonbox/server.rs::is_terminal_response()`. The server
/// treats `ContainerCreated` as non-terminal for ephemeral runs because it uses
/// a channel (`tx`) to communicate with the handler. For non-ephemeral runs, the
/// server drops `tx` after sending `ContainerCreated`, causing the handler loop
/// to naturally exit.
///
/// The proxy reads from a vsock stream and doesn't have a channel to drop. It
/// needs to know the ephemeral flag to distinguish: for non-ephemeral runs,
/// `ContainerCreated` signals the end (daemon will close the connection); for
/// ephemeral runs, it's just the start (streaming output follows).
fn is_terminal(resp: &DaemonResponse, is_ephemeral_run: bool) -> bool {
    match resp {
        // Always terminal
        DaemonResponse::ContainerStopped { .. }
        | DaemonResponse::Error { .. }
        | DaemonResponse::Success { .. }
        | DaemonResponse::ContainerList { .. } => true,

        // ContainerCreated is terminal only for non-ephemeral runs
        DaemonResponse::ContainerCreated { .. } => !is_ephemeral_run,

        // ContainerOutput is never terminal (only used in ephemeral runs)
        DaemonResponse::ContainerOutput { .. } => false,
    }
}

/// Proxy that sends requests to and receives responses from an in-VM daemon
/// over any [`AsyncRead`] + [`AsyncWrite`] stream (typically a vsock connection).
///
/// # Protocol
///
/// Each request is serialized as a single line of JSON followed by `\n`.
/// Responses are collected in the same format until a terminal response is received.
///
/// Streaming responses (e.g., `ContainerOutput`) can occur multiple times before
/// a terminal response (e.g., `ContainerStopped` or `Error`), and all are
/// collected in a single `Vec<DaemonResponse>`.
///
/// # Note on reusability
///
/// This proxy is designed for single-request usage. The generic parameter `S`
/// can be any type that implements both `AsyncRead` and `AsyncWrite`, including
/// those that support multiple sequential requests. For multiple requests, create
/// a new `VzProxy` instance for each request or manage the stream lifecycle externally.
pub struct VzProxy<S> {
    stream: S,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> VzProxy<S> {
    /// Create a new proxy over the given async stream.
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    /// Send a request and collect all responses until a terminal response.
    ///
    /// # Protocol flow
    ///
    /// 1. Serialize the request as JSON and append `\n`
    /// 2. Write the serialized request to the stream
    /// 3. Read responses line-by-line
    /// 4. Collect each response until `is_terminal()` returns `true`
    /// 5. Return all collected responses
    ///
    /// # Errors
    ///
    /// Returns an error if serialization, writing, reading, or deserialization fails.
    /// Also returns an error if the stream closes before a terminal response is received.
    pub async fn send_request(&mut self, req: &DaemonRequest) -> Result<Vec<DaemonResponse>> {
        // Serialize request to JSON
        let mut line = serde_json::to_string(req).context("serializing request")?;
        line.push('\n');

        // Write the request directly to the stream
        self.stream
            .write_all(line.as_bytes())
            .await
            .context("writing request to vsock")?;

        // Flush to ensure the request is sent
        self.stream
            .flush()
            .await
            .context("flushing request to vsock")?;

        // Check if this is an ephemeral Run request to guide terminal detection
        let is_ephemeral_run = matches!(
            req,
            DaemonRequest::Run {
                ephemeral: true,
                ..
            }
        );

        // Wrap stream in a buffered reader for line-by-line reading
        let mut buf_reader = BufReader::new(&mut self.stream);
        let mut responses = Vec::new();

        loop {
            let mut resp_line = String::new();
            let n = buf_reader
                .read_line(&mut resp_line)
                .await
                .context("reading response from vsock")?;

            // EOF without terminal response is an error
            if n == 0 {
                return Err(anyhow::anyhow!(
                    "vsock closed before terminal response received"
                ));
            }

            // Deserialize the response
            let resp: DaemonResponse =
                serde_json::from_str(resp_line.trim()).context("parsing response")?;

            let terminal = is_terminal(&resp, is_ephemeral_run);
            responses.push(resp);

            // Stop collecting responses once we get a terminal one
            if terminal {
                break;
            }
        }

        Ok(responses)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::protocol::OutputStreamKind;
    use tokio::io::duplex;

    #[tokio::test]
    async fn proxy_reads_single_terminal_response() {
        let (client, mut server) = duplex(1024);
        let resp = DaemonResponse::Success {
            message: "ok".into(),
        };
        let line = serde_json::to_string(&resp)
            .context("build response line")
            .unwrap()
            + "\n";

        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            server
                .write_all(line.as_bytes())
                .await
                .expect("write response");
        });

        let mut proxy = VzProxy::new(client);
        let req = DaemonRequest::List;
        let responses = proxy.send_request(&req).await.expect("send request");

        assert_eq!(responses.len(), 1);
        assert!(matches!(&responses[0], DaemonResponse::Success { .. }));
    }

    #[tokio::test]
    async fn proxy_collects_streaming_output() {
        let (client, mut server) = duplex(4096);

        // Build response sequence: ContainerCreated -> 2x ContainerOutput -> ContainerStopped
        let resp1 = DaemonResponse::ContainerCreated {
            id: "test-id".to_string(),
        };
        let resp2 = DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data: "aGVsbG8=".to_string(), // base64("hello")
        };
        let resp3 = DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data: "IHdvcmxk".to_string(), // base64(" world")
        };
        let resp4 = DaemonResponse::ContainerStopped { exit_code: 0 };

        let lines = vec![
            serde_json::to_string(&resp1).unwrap() + "\n",
            serde_json::to_string(&resp2).unwrap() + "\n",
            serde_json::to_string(&resp3).unwrap() + "\n",
            serde_json::to_string(&resp4).unwrap() + "\n",
        ];

        let _handle = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            for line in lines {
                server.write_all(line.as_bytes()).await.expect("write");
            }
        });

        let mut proxy = VzProxy::new(client);
        let req = DaemonRequest::Run {
            image: "alpine".to_string(),
            tag: None,
            command: vec!["/bin/echo".to_string(), "hello world".to_string()],
            memory_limit_bytes: None,
            cpu_weight: None,
            ephemeral: true,
            network: None,
            env: vec![],
            mounts: vec![],
            privileged: false,
        };
        let responses = proxy.send_request(&req).await.expect("send request");

        // Should have collected all 4 responses
        assert_eq!(responses.len(), 4);

        // Check first response is ContainerCreated
        assert!(matches!(
            &responses[0],
            DaemonResponse::ContainerCreated { .. }
        ));

        // Check streaming outputs
        match &responses[1] {
            DaemonResponse::ContainerOutput { stream, data } => {
                assert_eq!(stream, &OutputStreamKind::Stdout);
                assert_eq!(data, "aGVsbG8=");
            }
            _ => panic!("expected ContainerOutput at index 1"),
        }

        match &responses[2] {
            DaemonResponse::ContainerOutput { stream, data } => {
                assert_eq!(stream, &OutputStreamKind::Stdout);
                assert_eq!(data, "IHdvcmxk");
            }
            _ => panic!("expected ContainerOutput at index 2"),
        }

        // Check final response is terminal
        assert!(matches!(
            &responses[3],
            DaemonResponse::ContainerStopped { .. }
        ));
    }

    #[tokio::test]
    async fn proxy_handles_error_response() {
        let (client, mut server) = duplex(1024);
        let resp = DaemonResponse::Error {
            message: "container not found".to_string(),
        };
        let line = serde_json::to_string(&resp).unwrap() + "\n";

        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            server.write_all(line.as_bytes()).await.ok();
        });

        let mut proxy = VzProxy::new(client);
        let req = DaemonRequest::Stop {
            id: "nonexistent".to_string(),
        };
        let responses = proxy.send_request(&req).await.expect("send request");

        assert_eq!(responses.len(), 1);
        match &responses[0] {
            DaemonResponse::Error { message } => {
                assert_eq!(message, "container not found");
            }
            _ => panic!("expected Error response"),
        }
    }

    #[tokio::test]
    async fn proxy_handles_non_ephemeral_run_with_container_created() {
        let (client, mut server) = duplex(1024);

        // Spawn a task that sends ContainerCreated and closes (non-ephemeral Run)
        let _handle = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let resp = DaemonResponse::ContainerCreated {
                id: "test-id-123".to_string(),
            };
            let line = serde_json::to_string(&resp).unwrap() + "\n";
            server.write_all(line.as_bytes()).await.ok();
            // Server closes connection after sending ContainerCreated (non-ephemeral)
            drop(server);
        });

        let mut proxy = VzProxy::new(client);
        let req = DaemonRequest::Run {
            image: "alpine".to_string(),
            tag: None,
            command: vec!["/bin/sh".to_string()],
            memory_limit_bytes: None,
            cpu_weight: None,
            ephemeral: false, // Non-ephemeral run
            network: None,
            env: vec![],
            mounts: vec![],
            privileged: false,
        };
        let result = proxy.send_request(&req).await;

        // Should succeed and collect the single ContainerCreated response
        // (ContainerCreated is terminal for non-ephemeral runs)
        assert!(result.is_ok(), "Expected success, got error: {:?}", result);
        let responses = result.unwrap();
        assert_eq!(responses.len(), 1);
        assert!(matches!(
            &responses[0],
            DaemonResponse::ContainerCreated { id } if id == "test-id-123"
        ));
    }

    #[tokio::test]
    async fn proxy_closes_on_eof_without_terminal() {
        let (client, mut server) = duplex(1024);

        // Spawn a task that sends a non-terminal response then closes
        let _handle = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            // Send a ContainerOutput (non-terminal even in ephemeral runs)
            let resp = DaemonResponse::ContainerOutput {
                stream: OutputStreamKind::Stdout,
                data: "dGVzdA==".to_string(), // base64("test")
            };
            let line = serde_json::to_string(&resp).unwrap() + "\n";
            server.write_all(line.as_bytes()).await.ok();
            // Don't send a terminal response; just close
            drop(server);
        });

        let mut proxy = VzProxy::new(client);
        let req = DaemonRequest::Run {
            image: "alpine".to_string(),
            tag: None,
            command: vec!["/bin/sh".to_string()],
            memory_limit_bytes: None,
            cpu_weight: None,
            ephemeral: true,
            network: None,
            env: vec![],
            mounts: vec![],
            privileged: false,
        };
        let result = proxy.send_request(&req).await;

        // Should fail because ContainerOutput is non-terminal and we get EOF
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("vsock closed before terminal response"),
            "Expected error message to contain 'vsock closed before terminal response', got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn proxy_handles_list_response() {
        use minibox_core::protocol::ContainerInfo;

        let (client, mut server) = duplex(2048);
        let containers = vec![ContainerInfo {
            id: "test-123".to_string(),
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: "running".to_string(),
            created_at: "2026-03-28T00:00:00Z".to_string(),
            pid: Some(1234),
        }];

        let resp = DaemonResponse::ContainerList { containers };
        let line = serde_json::to_string(&resp).unwrap() + "\n";

        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            server.write_all(line.as_bytes()).await.ok();
        });

        let mut proxy = VzProxy::new(client);
        let req = DaemonRequest::List;
        let responses = proxy.send_request(&req).await.expect("send request");

        assert_eq!(responses.len(), 1);
        match &responses[0] {
            DaemonResponse::ContainerList { containers } => {
                assert_eq!(containers.len(), 1);
                assert_eq!(containers[0].id, "test-123");
            }
            _ => panic!("expected ContainerList response"),
        }
    }
}
