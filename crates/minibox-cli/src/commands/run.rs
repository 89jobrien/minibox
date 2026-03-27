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

use anyhow::{Context as _, Result};
use base64::Engine;
use minibox_client::DaemonClient;
use minibox_core::domain::{BindMount, NetworkMode};
use minibox_core::protocol::{DaemonRequest, DaemonResponse, OutputStreamKind};
use std::io::Write;
use std::path::PathBuf;

/// Parse a `-v src:dst[:ro]` volume shorthand into a `BindMount`.
pub fn parse_volume(s: &str) -> anyhow::Result<BindMount> {
    let parts: Vec<&str> = s.splitn(3, ':').collect();
    if parts.len() < 2 {
        anyhow::bail!(
            "invalid volume format {:?}: expected src:dst or src:dst:ro",
            s
        );
    }
    let host_path = PathBuf::from(parts[0]);
    let container_path = PathBuf::from(parts[1]);
    if !container_path.is_absolute() {
        anyhow::bail!(
            "container path {:?} must be absolute (start with /)",
            container_path
        );
    }
    let read_only = parts.get(2).map(|f| *f == "ro").unwrap_or(false);
    Ok(BindMount {
        host_path,
        container_path,
        read_only,
    })
}

/// Parse a `--mount type=bind,src=PATH,dst=PATH[,readonly]` spec into a `BindMount`.
pub fn parse_mount(s: &str) -> anyhow::Result<BindMount> {
    let mut mount_type = None::<String>;
    let mut src = None::<PathBuf>;
    let mut dst = None::<PathBuf>;
    let mut read_only = false;

    for kv in s.split(',') {
        if kv == "readonly" || kv == "ro" {
            read_only = true;
            continue;
        }
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        match k {
            "type" => mount_type = Some(v.to_string()),
            "src" | "source" => src = Some(PathBuf::from(v)),
            "dst" | "target" | "destination" => dst = Some(PathBuf::from(v)),
            _ => {}
        }
    }

    match mount_type.as_deref() {
        Some("bind") | None => {}
        Some(t) => anyhow::bail!("unsupported mount type {:?}: only 'bind' is supported", t),
    }

    let host_path = src.ok_or_else(|| anyhow::anyhow!("--mount missing 'src' key"))?;
    let container_path = dst.ok_or_else(|| anyhow::anyhow!("--mount missing 'dst' key"))?;
    if !container_path.is_absolute() {
        anyhow::bail!(
            "container path {:?} must be absolute (start with /)",
            container_path
        );
    }

    Ok(BindMount {
        host_path,
        container_path,
        read_only,
    })
}

/// Execute the `run` subcommand.
///
/// Connects to the daemon, sends an ephemeral `DaemonRequest::Run`, then
/// streams `ContainerOutput` chunks to stdout/stderr until `ContainerStopped`
/// is received.  Exits with the container's exit code.
#[allow(clippy::too_many_arguments)]
pub async fn execute(
    image: String,
    tag: String,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    network: String,
    privileged: bool,
    volumes: Vec<String>,
    mount_specs: Vec<String>,
    socket_path: &std::path::Path,
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

    // Parse -v shorthand mounts.
    let mut mounts: Vec<BindMount> = Vec::new();
    for v in &volumes {
        mounts.push(parse_volume(v).with_context(|| format!("invalid -v flag {:?}", v))?);
    }
    // Parse --mount long-form mounts.
    for m in &mount_specs {
        mounts.push(parse_mount(m).with_context(|| format!("invalid --mount flag {:?}", m))?);
    }

    let request = DaemonRequest::Run {
        image,
        tag: Some(tag),
        command,
        memory_limit_bytes,
        cpu_weight,
        ephemeral: true,
        network: Some(network_mode),
        mounts,
        privileged,
    };

    let client = DaemonClient::with_socket(socket_path);
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
                // Print the container ID for the caller.  In the ephemeral
                // streaming path this is the first message; ContainerOutput
                // chunks and ContainerStopped follow.  In the non-ephemeral
                // path the daemon closes the channel after this message, so
                // the while-loop exits naturally.
                println!("{id}");
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
    use minibox_core::protocol::{DaemonResponse, OutputStreamKind};

    #[cfg(unix)]
    async fn serve_once(socket_path: &std::path::Path, response: DaemonResponse) {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixListener;
        let listener = UnixListener::bind(socket_path).unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let mut resp = serde_json::to_string(&response).unwrap();
        resp.push('\n');
        write_half.write_all(resp.as_bytes()).await.unwrap();
        write_half.flush().await.unwrap();
    }

    /// ContainerCreated is the non-streaming legacy path — returns Ok(()) without
    /// calling process::exit, so it's the only execute() path testable in-process.
    #[cfg(unix)]
    #[tokio::test]
    async fn execute_returns_ok_on_container_created() {
        let tmp = tempfile::TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");
        let sp = socket_path.clone();
        tokio::spawn(async move {
            serve_once(
                &sp,
                DaemonResponse::ContainerCreated {
                    id: "abc123".to_string(),
                },
            )
            .await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let result = execute(
            "alpine".to_string(),
            "latest".to_string(),
            vec!["/bin/sh".to_string()],
            None,
            None,
            "none".to_string(),
            false,
            vec![],
            vec![],
            &socket_path,
        )
        .await;
        assert!(
            result.is_ok(),
            "execute should return Ok on ContainerCreated: {result:?}"
        );
    }

    fn network_mode_from_str(s: &str) -> Result<NetworkMode> {
        match s {
            "none" => Ok(NetworkMode::None),
            "bridge" => Ok(NetworkMode::Bridge),
            "host" => Ok(NetworkMode::Host),
            "tailnet" => Ok(NetworkMode::Tailnet),
            other => anyhow::bail!(
                "unknown network mode: {other} (expected: none, bridge, host, tailnet)"
            ),
        }
    }

    #[test]
    fn network_mode_none() {
        assert!(matches!(
            network_mode_from_str("none").unwrap(),
            NetworkMode::None
        ));
    }

    #[test]
    fn network_mode_bridge() {
        assert!(matches!(
            network_mode_from_str("bridge").unwrap(),
            NetworkMode::Bridge
        ));
    }

    #[test]
    fn network_mode_host() {
        assert!(matches!(
            network_mode_from_str("host").unwrap(),
            NetworkMode::Host
        ));
    }

    #[test]
    fn network_mode_tailnet() {
        assert!(matches!(
            network_mode_from_str("tailnet").unwrap(),
            NetworkMode::Tailnet
        ));
    }

    #[test]
    fn network_mode_unknown_errors() {
        let err = network_mode_from_str("docker").unwrap_err();
        assert!(
            err.to_string().contains("unknown network mode"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parse_volume_valid_rw() {
        let m = parse_volume("/tmp/host:/guest").unwrap();
        assert_eq!(m.host_path, PathBuf::from("/tmp/host"));
        assert_eq!(m.container_path, PathBuf::from("/guest"));
        assert!(!m.read_only);
    }

    #[test]
    fn parse_volume_valid_ro() {
        let m = parse_volume("/tmp/host:/guest:ro").unwrap();
        assert_eq!(m.host_path, PathBuf::from("/tmp/host"));
        assert_eq!(m.container_path, PathBuf::from("/guest"));
        assert!(m.read_only);
    }

    #[test]
    fn parse_volume_missing_colon_errors() {
        let err = parse_volume("/tmp/nocolon").unwrap_err();
        assert!(err.to_string().contains(":"), "expected colon hint: {err}");
    }

    #[test]
    fn parse_volume_relative_dst_errors() {
        let err = parse_volume("/tmp/host:relative/path").unwrap_err();
        assert!(
            err.to_string().contains("absolute") || err.to_string().contains("/"),
            "expected absolute path error: {err}"
        );
    }

    #[test]
    fn parse_mount_valid_bind() {
        let m = parse_mount("type=bind,src=/tmp/host,dst=/guest").unwrap();
        assert_eq!(m.host_path, PathBuf::from("/tmp/host"));
        assert_eq!(m.container_path, PathBuf::from("/guest"));
        assert!(!m.read_only);
    }

    #[test]
    fn parse_mount_readonly() {
        let m = parse_mount("type=bind,src=/tmp/host,dst=/guest,readonly").unwrap();
        assert!(m.read_only);
    }

    #[test]
    fn parse_mount_non_bind_type_errors() {
        let err = parse_mount("type=volume,src=myvolume,dst=/data").unwrap_err();
        assert!(
            err.to_string().contains("bind") || err.to_string().contains("type"),
            "expected bind-only error: {err}"
        );
    }

    #[test]
    fn parse_mount_missing_src_errors() {
        let err = parse_mount("type=bind,dst=/guest").unwrap_err();
        assert!(err.to_string().contains("src"), "expected src error: {err}");
    }

    #[test]
    fn parse_mount_missing_dst_errors() {
        let err = parse_mount("type=bind,src=/tmp/host").unwrap_err();
        assert!(err.to_string().contains("dst"), "expected dst error: {err}");
    }

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
