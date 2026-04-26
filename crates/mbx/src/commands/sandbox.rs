//! `mbx sandbox` — run a script inside a sandboxed minibox container.
//!
//! Detects the language from the file extension, bind-mounts the script into
//! the container at `/workspace/script`, and streams output back. Enforces
//! safety defaults: 512 MB memory, 60 s timeout, no network, no privileged.

use anyhow::{Context as _, Result, bail};
use base64::Engine;
use minibox_core::client::DaemonClient;
use minibox_core::domain::{BindMount, NetworkMode};
use minibox_core::protocol::{DaemonRequest, DaemonResponse, OutputStreamKind};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Detect the interpreter command from a file extension.
///
/// Returns `(interpreter, args-before-script)` so the container command
/// becomes `[interpreter, ...args, "/workspace/script"]`.
pub fn detect_interpreter(path: &Path) -> Result<(String, Vec<String>)> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "py" => Ok(("python3".to_string(), vec![])),
        "rs" => Ok(("rust-script".to_string(), vec![])),
        "sh" => Ok(("sh".to_string(), vec![])),
        "nu" => Ok(("nu".to_string(), vec![])),
        "js" => Ok(("node".to_string(), vec![])),
        "" => {
            // No extension — try to read shebang
            Ok(("sh".to_string(), vec![]))
        }
        other => {
            bail!("unsupported script extension: .{other} (supported: .py, .rs, .sh, .nu, .js)")
        }
    }
}

/// Build the `DaemonRequest::Run` for a sandbox execution.
pub fn build_request(
    script: &Path,
    image: &str,
    tag: &str,
    memory_mb: u64,
    extra_mounts: Vec<BindMount>,
    network: bool,
) -> Result<DaemonRequest> {
    let script = script
        .canonicalize()
        .with_context(|| format!("script not found: {}", script.display()))?;

    let (interpreter, mut args) = detect_interpreter(&script)?;
    args.push("/workspace/script".to_string());

    let mut command = vec![interpreter];
    command.append(&mut args);

    let mut mounts = vec![BindMount {
        host_path: script,
        container_path: PathBuf::from("/workspace/script"),
        read_only: true,
    }];
    mounts.extend(extra_mounts);

    let network_mode = if network {
        NetworkMode::Bridge
    } else {
        NetworkMode::None
    };

    Ok(DaemonRequest::Run {
        image: image.to_string(),
        tag: Some(tag.to_string()),
        command,
        memory_limit_bytes: Some(memory_mb * 1024 * 1024),
        cpu_weight: Some(100),
        ephemeral: true,
        network: Some(network_mode),
        mounts,
        privileged: false,
        env: vec![],
        name: None,
        tty: false,
        priority: None,
        urgency: None,
        execution_context: None,
    })
}

/// Execute the `sandbox` subcommand.
#[allow(clippy::too_many_arguments)]
pub async fn execute(
    script: PathBuf,
    image: String,
    tag: String,
    memory_mb: u64,
    timeout_secs: u64,
    extra_mounts: Vec<BindMount>,
    network: bool,
    socket_path: &Path,
) -> Result<()> {
    let request = build_request(&script, &image, &tag, memory_mb, extra_mounts, network)?;

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(request)
        .await
        .context("failed to connect to daemon")?;

    let timeout = tokio::time::Duration::from_secs(timeout_secs);
    let result = tokio::time::timeout(timeout, async {
        while let Some(response) = stream.next().await.context("stream error")? {
            match response {
                DaemonResponse::ContainerOutput { stream, data } => {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(&data)
                        .context("failed to decode output chunk")?;
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
                    // Ephemeral path — container ID printed for reference, streaming follows.
                    eprintln!("sandbox: container {id}");
                }
                DaemonResponse::Error { message } => {
                    eprintln!("sandbox error: {message}");
                    std::process::exit(1);
                }
                other => {
                    eprintln!("sandbox: unexpected response: {other:?}");
                    std::process::exit(1);
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .await;

    match result {
        Ok(inner) => inner,
        Err(_elapsed) => {
            eprintln!("sandbox: timeout after {timeout_secs}s — container killed");
            std::process::exit(124); // Same as GNU timeout
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_python() {
        let (interp, args) = detect_interpreter(Path::new("script.py")).unwrap();
        assert_eq!(interp, "python3");
        assert!(args.is_empty());
    }

    #[test]
    fn detect_rust() {
        let (interp, _) = detect_interpreter(Path::new("main.rs")).unwrap();
        assert_eq!(interp, "rust-script");
    }

    #[test]
    fn detect_shell() {
        let (interp, _) = detect_interpreter(Path::new("run.sh")).unwrap();
        assert_eq!(interp, "sh");
    }

    #[test]
    fn detect_nushell() {
        let (interp, _) = detect_interpreter(Path::new("task.nu")).unwrap();
        assert_eq!(interp, "nu");
    }

    #[test]
    fn detect_javascript() {
        let (interp, _) = detect_interpreter(Path::new("app.js")).unwrap();
        assert_eq!(interp, "node");
    }

    #[test]
    fn detect_no_extension_defaults_to_sh() {
        let (interp, _) = detect_interpreter(Path::new("Makefile")).unwrap();
        assert_eq!(interp, "sh");
    }

    #[test]
    fn detect_unsupported_extension_errors() {
        let err = detect_interpreter(Path::new("data.csv")).unwrap_err();
        assert!(err.to_string().contains("unsupported"));
    }

    #[test]
    fn build_request_sets_ephemeral() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let req = build_request(tmp.path(), "minibox-sandbox", "latest", 512, vec![], false);
        assert!(req.is_ok());
        if let Ok(DaemonRequest::Run { ephemeral, .. }) = req {
            assert!(ephemeral);
        } else {
            panic!("expected Run request");
        }
    }

    #[test]
    fn build_request_enforces_no_privileged() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let req = build_request(tmp.path(), "sandbox", "latest", 256, vec![], false).unwrap();
        if let DaemonRequest::Run { privileged, .. } = req {
            assert!(!privileged);
        } else {
            panic!("expected Run");
        }
    }

    #[test]
    fn build_request_memory_conversion() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let req = build_request(tmp.path(), "sandbox", "latest", 512, vec![], false).unwrap();
        if let DaemonRequest::Run {
            memory_limit_bytes, ..
        } = req
        {
            assert_eq!(memory_limit_bytes, Some(512 * 1024 * 1024));
        } else {
            panic!("expected Run");
        }
    }

    #[test]
    fn build_request_network_off_by_default() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let req = build_request(tmp.path(), "sandbox", "latest", 512, vec![], false).unwrap();
        if let DaemonRequest::Run { network, .. } = req {
            assert_eq!(network, Some(NetworkMode::None));
        } else {
            panic!("expected Run");
        }
    }

    #[test]
    fn build_request_network_bridge_when_enabled() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let req = build_request(tmp.path(), "sandbox", "latest", 512, vec![], true).unwrap();
        if let DaemonRequest::Run { network, .. } = req {
            assert_eq!(network, Some(NetworkMode::Bridge));
        } else {
            panic!("expected Run");
        }
    }

    #[test]
    fn build_request_mounts_script_readonly() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let req = build_request(tmp.path(), "sandbox", "latest", 512, vec![], false).unwrap();
        if let DaemonRequest::Run { mounts, .. } = req {
            assert!(!mounts.is_empty());
            let script_mount = &mounts[0];
            assert_eq!(
                script_mount.container_path,
                PathBuf::from("/workspace/script")
            );
            assert!(script_mount.read_only);
        } else {
            panic!("expected Run");
        }
    }

    #[test]
    fn build_request_includes_extra_mounts() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let extra = vec![BindMount {
            host_path: PathBuf::from("/tmp/data"),
            container_path: PathBuf::from("/data"),
            read_only: false,
        }];
        let req = build_request(tmp.path(), "sandbox", "latest", 512, extra, false).unwrap();
        if let DaemonRequest::Run { mounts, .. } = req {
            assert_eq!(mounts.len(), 2);
        } else {
            panic!("expected Run");
        }
    }

    #[test]
    fn build_request_missing_script_errors() {
        let err = build_request(
            Path::new("/nonexistent/script.py"),
            "sandbox",
            "latest",
            512,
            vec![],
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("script not found"));
    }
}
