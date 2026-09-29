//! `mbx build` — build an image from a Dockerfile context.

use super::RequestError;
use anyhow::{Context as _, Result, bail};
use minibox_core::client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use std::path::{Path, PathBuf};

/// Options accepted by the image build command.
#[derive(Debug)]
pub struct BuildOptions {
    /// Build context directory.
    pub context: PathBuf,
    /// Target image tag.
    pub tag: String,
    /// Dockerfile path relative to the build context.
    pub file: PathBuf,
    /// Build-time variables in `KEY=VALUE` form.
    pub build_args: Vec<String>,
    /// Whether cached layers should be ignored.
    pub no_cache: bool,
}

fn protocol_path(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .context("build context path is not valid UTF-8")
}

/// Execute the `build` subcommand.
pub async fn execute(options: BuildOptions, socket_path: &Path) -> Result<()> {
    let context = options
        .context
        .canonicalize()
        .with_context(|| format!("invalid build context: {}", options.context.display()))?;
    if !context.is_dir() {
        bail!("build context is not a directory: {}", context.display());
    }
    if options.file.is_absolute() {
        bail!("Dockerfile path must be relative to the build context");
    }

    let dockerfile_path = context
        .join(&options.file)
        .canonicalize()
        .with_context(|| format!("invalid Dockerfile: {}", options.file.display()))?;
    if !dockerfile_path.starts_with(&context) {
        bail!("Dockerfile must be inside the build context");
    }

    let dockerfile = tokio::fs::read_to_string(&dockerfile_path)
        .await
        .with_context(|| format!("read Dockerfile: {}", dockerfile_path.display()))?;
    let build_args = options
        .build_args
        .into_iter()
        .map(|arg| {
            let (key, value) = arg
                .split_once('=')
                .with_context(|| format!("invalid build argument {arg:?}; expected KEY=VALUE"))?;
            if key.is_empty() {
                bail!("invalid build argument {arg:?}; key cannot be empty");
            }
            Ok((key.to_string(), value.to_string()))
        })
        .collect::<Result<Vec<_>>>()?;

    let request = DaemonRequest::Build {
        dockerfile,
        context_path: protocol_path(&context)?,
        tag: options.tag,
        build_args,
        no_cache: options.no_cache,
    };
    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    while let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::BuildOutput {
                step,
                total_steps,
                message,
            } => println!("[{step}/{total_steps}] {message}"),
            DaemonResponse::BuildComplete { image_id, tag } => {
                println!("Successfully built {image_id}");
                println!("Successfully tagged {tag}");
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
    use super::*;
    use minibox_core::protocol::{DaemonRequest, DaemonResponse};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_sends_build_request_and_accepts_streamed_completion() {
        let temp_dir = tempfile::TempDir::new().expect("create temp dir");
        let context = temp_dir.path().join("context");
        std::fs::create_dir(&context).expect("create build context");
        std::fs::write(context.join("Dockerfile"), "FROM scratch\n").expect("write Dockerfile");

        let socket_path = temp_dir.path().join("build.sock");
        let server_socket = socket_path.clone();
        let server = tokio::spawn(async move {
            let listener = UnixListener::bind(server_socket).expect("bind test socket");
            let (stream, _) = listener.accept().await.expect("accept connection");
            let (read_half, mut write_half) = tokio::io::split(stream);
            let mut reader = BufReader::new(read_half);
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read request");

            for response in [
                DaemonResponse::BuildOutput {
                    step: 1,
                    total_steps: 1,
                    message: "FROM scratch".to_string(),
                },
                DaemonResponse::BuildComplete {
                    image_id: "sha256:test".to_string(),
                    tag: "example:test".to_string(),
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
            BuildOptions {
                context: context.clone(),
                tag: "example:test".to_string(),
                file: PathBuf::from("Dockerfile"),
                build_args: vec!["GREETING=hello=world".to_string()],
                no_cache: true,
            },
            &socket_path,
        )
        .await;

        assert!(result.is_ok(), "build should succeed: {result:?}");
        let request = server.await.expect("server task");
        match request {
            DaemonRequest::Build {
                dockerfile,
                context_path,
                tag,
                build_args,
                no_cache,
            } => {
                assert_eq!(dockerfile, "FROM scratch\n");
                assert_eq!(
                    context_path,
                    context.canonicalize().unwrap().to_string_lossy()
                );
                assert_eq!(tag, "example:test");
                assert_eq!(
                    build_args,
                    vec![("GREETING".to_string(), "hello=world".to_string())]
                );
                assert!(no_cache);
            }
            other => panic!("expected Build request, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn protocol_path_rejects_non_utf8_input() {
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(std::ffi::OsString::from_vec(b"context-\xff".to_vec()));
        let error = protocol_path(&path).expect_err("non-UTF-8 context should fail");
        assert!(
            error.to_string().contains("not valid UTF-8"),
            "unexpected error: {error:#}"
        );
    }
}
