//! `mbx push` — push a locally-stored image to a remote OCI registry.

use super::RequestError;
use anyhow::Context as _;
use minibox_core::client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse, PushCredentials};
use std::path::Path;

/// Registry credentials supplied on the command line.
///
/// All three fields are optional; the combination selects which
/// [`PushCredentials`] variant is sent to the daemon.
#[derive(Debug, Default, Clone)]
pub struct PushAuth {
    /// Registry account username.
    pub username: Option<String>,
    /// Registry account password.
    pub password: Option<String>,
    /// Bearer token, used instead of a username/password pair.
    pub token: Option<String>,
}

/// Select the [`PushCredentials`] variant implied by the supplied auth fields.
///
/// # Errors
///
/// Returns an error when the combination is ambiguous or incomplete:
/// `--password` without `--username`, `--token` combined with
/// `--username`/`--password`, or a blank `--username`.
pub fn resolve_credentials(auth: &PushAuth) -> anyhow::Result<PushCredentials> {
    if let Some(token) = auth.token.as_deref() {
        if auth.username.is_some() || auth.password.is_some() {
            anyhow::bail!("--token cannot be combined with --username or --password");
        }
        return Ok(PushCredentials::Token {
            token: token.to_string(),
        });
    }

    match (&auth.username, &auth.password) {
        (None, None) => Ok(PushCredentials::Anonymous),
        (Some(username), Some(password)) => {
            if username.is_empty() {
                anyhow::bail!("--username cannot be empty");
            }
            Ok(PushCredentials::Basic {
                username: username.clone(),
                password: password.clone(),
            })
        }
        _ => anyhow::bail!("--password requires --username"),
    }
}

/// Format a single `PushProgress` line for terminal output.
#[must_use]
pub fn format_progress(digest: &str, bytes_uploaded: u64, total_bytes: u64) -> String {
    match bytes_uploaded.saturating_mul(100).checked_div(total_bytes) {
        Some(percent) => format!("{digest}: {bytes_uploaded}/{total_bytes} bytes ({percent}%)"),
        None => format!("{digest}: {bytes_uploaded} bytes uploaded"),
    }
}

/// Execute the `push` subcommand.
///
/// Resolves the registry credentials, sends `DaemonRequest::Push`, then streams
/// `PushProgress` updates until the daemon returns a terminal `Success` or
/// `Error`.  Daemon failures propagate as [`RequestError`], which the CLI turns
/// into a non-zero exit.
pub async fn execute(image_ref: String, auth: PushAuth, socket_path: &Path) -> anyhow::Result<()> {
    let credentials = resolve_credentials(&auth)?;

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(DaemonRequest::Push {
            image_ref,
            credentials,
        })
        .await
        .context("failed to call daemon")?;

    while let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::PushProgress {
                layer_digest,
                bytes_uploaded,
                total_bytes,
            } => {
                println!(
                    "{}",
                    format_progress(&layer_digest, bytes_uploaded, total_bytes)
                );
            }
            DaemonResponse::Success { message } => {
                println!("{message}");
                return Ok(());
            }
            DaemonResponse::Error { message } => {
                return Err(RequestError::DaemonError { message }.into());
            }
            other => {
                return Err(RequestError::UnexpectedResponse {
                    response: format!("{other:?}"),
                }
                .into());
            }
        }
    }

    Err(RequestError::NoResponse.into())
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::setup_multi;
    use super::*;
    use minibox_core::protocol::{DaemonRequest, DaemonResponse};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    #[test]
    fn resolve_credentials_defaults_to_anonymous() {
        let creds = resolve_credentials(&PushAuth::default()).expect("anonymous");
        assert!(
            matches!(creds, PushCredentials::Anonymous),
            "expected Anonymous, got: {creds:?}"
        );
    }

    #[test]
    fn resolve_credentials_builds_basic_from_username_and_password() {
        let auth = PushAuth {
            username: Some("alice".to_string()),
            password: Some("hunter2".to_string()),
            token: None,
        };
        let creds = resolve_credentials(&auth).expect("basic");
        match creds {
            PushCredentials::Basic { username, password } => {
                assert_eq!(username, "alice");
                assert_eq!(password, "hunter2");
            }
            other => panic!("expected Basic, got: {other:?}"),
        }
    }

    #[test]
    fn resolve_credentials_builds_token_from_token_flag() {
        let auth = PushAuth {
            username: None,
            password: None,
            token: Some("abc123".to_string()),
        };
        let creds = resolve_credentials(&auth).expect("token");
        match creds {
            PushCredentials::Token { token } => assert_eq!(token, "abc123"),
            other => panic!("expected Token, got: {other:?}"),
        }
    }

    #[test]
    fn resolve_credentials_rejects_password_without_username() {
        let auth = PushAuth {
            username: None,
            password: Some("hunter2".to_string()),
            token: None,
        };
        let error = resolve_credentials(&auth).expect_err("password alone must fail");
        assert!(
            error.to_string().contains("--password requires --username"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn resolve_credentials_rejects_token_combined_with_username() {
        let auth = PushAuth {
            username: Some("alice".to_string()),
            password: Some("hunter2".to_string()),
            token: Some("abc123".to_string()),
        };
        let error = resolve_credentials(&auth).expect_err("token + basic must fail");
        assert!(
            error.to_string().contains("cannot be combined"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn resolve_credentials_rejects_empty_username() {
        let auth = PushAuth {
            username: Some(String::new()),
            password: Some("hunter2".to_string()),
            token: None,
        };
        let error = resolve_credentials(&auth).expect_err("empty username must fail");
        assert!(
            error.to_string().contains("--username cannot be empty"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn format_progress_renders_byte_counts_and_percent() {
        let line = format_progress("sha256:abc", 512, 1024);
        assert_eq!(line, "sha256:abc: 512/1024 bytes (50%)");
    }

    #[test]
    fn format_progress_omits_percent_when_total_is_zero() {
        let line = format_progress("sha256:abc", 128, 0);
        assert_eq!(line, "sha256:abc: 128 bytes uploaded");
    }

    #[test]
    fn format_progress_clamps_percent_when_uploaded_exceeds_total() {
        let line = format_progress("sha256:abc", 2048, 1024);
        assert!(line.contains("(200%)"), "unexpected line: {line}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_streams_push_progress_then_success() {
        let temp_dir = tempfile::TempDir::new().expect("create temp dir");
        let socket_path = temp_dir.path().join("push.sock");
        let server_socket = socket_path.clone();
        let server = tokio::spawn(async move {
            let listener = UnixListener::bind(server_socket).expect("bind test socket");
            let (stream, _) = listener.accept().await.expect("accept connection");
            let (read_half, mut write_half) = tokio::io::split(stream);
            let mut reader = BufReader::new(read_half);
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read request");

            for response in [
                DaemonResponse::PushProgress {
                    layer_digest: "sha256:abc".to_string(),
                    bytes_uploaded: 512,
                    total_bytes: 1024,
                },
                DaemonResponse::Success {
                    message: "pushed myapp:latest".to_string(),
                },
            ] {
                let mut encoded = serde_json::to_string(&response).expect("encode response");
                encoded.push('\n');
                write_half
                    .write_all(encoded.as_bytes())
                    .await
                    .expect("write response");
            }
            write_half.flush().await.expect("flush responses");
            serde_json::from_str::<DaemonRequest>(&line).expect("decode request")
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let result = execute(
            "myapp:latest".to_string(),
            PushAuth {
                username: Some("alice".to_string()),
                password: Some("hunter2".to_string()),
                token: None,
            },
            &socket_path,
        )
        .await;
        assert!(result.is_ok(), "push should succeed: {result:?}");

        let request = server.await.expect("server task");
        match request {
            DaemonRequest::Push {
                image_ref,
                credentials,
            } => {
                assert_eq!(image_ref, "myapp:latest");
                assert!(
                    matches!(credentials, PushCredentials::Basic { .. }),
                    "expected Basic credentials, got: {credentials:?}"
                );
            }
            other => panic!("expected Push request, got: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_succeeds_without_progress_updates() {
        let (_tmp, socket_path) = setup_multi(vec![DaemonResponse::Success {
            message: "pushed myapp:latest".to_string(),
        }])
        .await;
        let result = execute(
            "myapp:latest".to_string(),
            PushAuth::default(),
            &socket_path,
        )
        .await;
        assert!(result.is_ok(), "push should succeed: {result:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_propagates_daemon_error() {
        let (_tmp, socket_path) = setup_multi(vec![DaemonResponse::Error {
            message: "registry rejected the manifest".to_string(),
        }])
        .await;
        let result = execute(
            "myapp:latest".to_string(),
            PushAuth::default(),
            &socket_path,
        )
        .await;
        let error = result.expect_err("daemon error should propagate");
        assert!(
            error.to_string().contains("registry rejected the manifest"),
            "error should carry daemon message, got: {error:#}"
        );
    }
}
