//! CNI plugin exec protocol: spawns a plugin binary per the CNI spec,
//! passing config over stdin and reading the JSON result from stdout.

use crate::config::PluginConfig;
use crate::error::CniError;
use crate::result::CniErrorPayload;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::{Span, instrument};

/// Locate a plugin binary by name on the given `CNI_PATH` directories.
fn find_plugin_binary(cni_path: &[PathBuf], plugin_type: &str) -> Result<PathBuf, CniError> {
    for dir in cni_path {
        let candidate = dir.join(plugin_type);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(CniError::PluginNotFound {
        plugin: plugin_type.to_string(),
        searched: cni_path.to_vec(),
    })
}

/// Invoke a single CNI plugin with the given command (`ADD`/`DEL`/`CHECK`).
///
/// Returns the plugin's JSON result on success. On failure, distinguishes
/// a CNI-spec structured error payload from a raw process failure and
/// records both as span fields (in addition to the automatic
/// `#[instrument(err)]` summary).
///
/// # Errors
///
/// Returns [`CniError::PluginNotFound`] if the plugin binary isn't on
/// `cni_path`, [`CniError::PluginError`] if the plugin returned a CNI-spec
/// structured error, [`CniError::ProcessFailed`] if it exited non-zero
/// without one, or [`CniError::Io`]/[`CniError::ConfigParse`] for
/// process-spawn or JSON errors.
#[instrument(
    skip(cni_path, plugin, prev_result),
    fields(
        plugin = %plugin.plugin_type,
        command = %command,
        exit_code = tracing::field::Empty,
        cni_error_code = tracing::field::Empty,
        cni_error_msg = tracing::field::Empty,
        stderr = tracing::field::Empty,
    ),
    err
)]
pub(crate) async fn exec_plugin(
    cni_path: &[PathBuf],
    plugin: &PluginConfig,
    command: &str,
    netns: &str,
    container_id: &str,
    ifname: &str,
    prev_result: Option<&serde_json::Value>,
) -> Result<serde_json::Value, CniError> {
    let binary = find_plugin_binary(cni_path, &plugin.plugin_type)?;

    let mut config = plugin.raw.clone();
    if let (Some(obj), Some(prev)) = (config.as_object_mut(), prev_result) {
        obj.insert("prevResult".to_string(), prev.clone());
    }
    let config_bytes = serde_json::to_vec(&config)?;

    let cni_path_joined =
        std::env::join_paths(cni_path).map_err(|e| CniError::Io(std::io::Error::other(e)))?;

    let mut child = Command::new(&binary)
        .env("CNI_COMMAND", command)
        .env("CNI_CONTAINERID", container_id)
        .env("CNI_NETNS", netns)
        .env("CNI_IFNAME", ifname)
        .env("CNI_PATH", &cni_path_joined)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| CniError::Io(std::io::Error::other("child stdin was not piped")))?;
    // A plugin may exit (and close its stdin) before consuming the config we
    // write, e.g. when it fails fast with a structured error. That produces
    // BrokenPipe here, which is not a real failure — the plugin's actual
    // exit status and output are read below and take precedence.
    if let Err(err) = stdin.write_all(&config_bytes).await
        && err.kind() != std::io::ErrorKind::BrokenPipe
    {
        return Err(CniError::Io(err));
    }
    drop(stdin);

    let output = child.wait_with_output().await?;
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        if let Ok(payload) = serde_json::from_slice::<CniErrorPayload>(&output.stdout) {
            Span::current().record("cni_error_code", payload.code);
            Span::current().record("cni_error_msg", payload.msg.as_str());
            Span::current().record("stderr", stderr.as_str());
            return Err(CniError::PluginError {
                plugin: plugin.plugin_type.clone(),
                code: Some(payload.code),
                msg: payload.msg,
                details: payload.details,
            });
        }

        Span::current().record("exit_code", output.status.code());
        Span::current().record("stderr", stderr.as_str());
        return Err(CniError::ProcessFailed {
            plugin: plugin.plugin_type.clone(),
            exit_code: output.status.code(),
            stderr,
        });
    }

    if output.stdout.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    serde_json::from_slice(&output.stdout).map_err(CniError::ConfigParse)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;

    /// Write an executable shell-script fixture plugin into `dir` that
    /// echoes canned JSON on ADD and exits 0 with no output on DEL.
    fn write_fixture_plugin(
        dir: &std::path::Path,
        name: &str,
        add_json: &str,
    ) -> std::path::PathBuf {
        let path = dir.join(name);
        let script = format!(
            "#!/bin/sh\nif [ \"$CNI_COMMAND\" = \"ADD\" ]; then\n  echo '{add_json}'\nfi\nexit 0\n"
        );
        std::fs::write(&path, script).expect("write fixture plugin");
        let mut perms = std::fs::metadata(&path)
            .expect("stat fixture plugin")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod fixture plugin");
        path
    }

    #[tokio::test]
    async fn exec_plugin_add_returns_parsed_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_fixture_plugin(
            dir.path(),
            "fake-bridge",
            "{\"cniVersion\":\"1.0.0\",\"interfaces\":[{\"name\":\"eth0\"}]}",
        );
        let plugin = PluginConfig {
            plugin_type: "fake-bridge".to_string(),
            raw: json!({"type": "fake-bridge"}),
        };

        let result = exec_plugin(
            &[dir.path().to_path_buf()],
            &plugin,
            "ADD",
            "/fake/netns",
            "container-1",
            "eth0",
            None,
        )
        .await
        .expect("exec_plugin should succeed");

        assert_eq!(result["cniVersion"], "1.0.0");
        assert_eq!(result["interfaces"][0]["name"], "eth0");
    }

    #[tokio::test]
    async fn exec_plugin_missing_binary_returns_plugin_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plugin = PluginConfig {
            plugin_type: "does-not-exist".to_string(),
            raw: json!({"type": "does-not-exist"}),
        };

        let result = exec_plugin(
            &[dir.path().to_path_buf()],
            &plugin,
            "ADD",
            "/fake/netns",
            "container-1",
            "eth0",
            None,
        )
        .await;

        assert!(matches!(result, Err(CniError::PluginNotFound { .. })));
    }

    #[tokio::test]
    async fn exec_plugin_structured_error_returns_plugin_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fake-host-local");
        let script = "#!/bin/sh\necho '{\"code\":7,\"msg\":\"no IPs available\",\"details\":\"pool exhausted\"}'\nexit 1\n";
        std::fs::write(&path, script).expect("write fixture plugin");
        let mut perms = std::fs::metadata(&path).expect("stat").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");

        let plugin = PluginConfig {
            plugin_type: "fake-host-local".to_string(),
            raw: json!({"type": "fake-host-local"}),
        };

        let result = exec_plugin(
            &[dir.path().to_path_buf()],
            &plugin,
            "ADD",
            "/fake/netns",
            "container-1",
            "eth0",
            None,
        )
        .await;

        match result {
            Err(CniError::PluginError {
                plugin,
                code,
                msg,
                details,
            }) => {
                assert_eq!(plugin, "fake-host-local");
                assert_eq!(code, Some(7));
                assert_eq!(msg, "no IPs available");
                assert_eq!(details.as_deref(), Some("pool exhausted"));
            }
            other => panic!("expected PluginError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn exec_plugin_crash_without_structured_error_returns_process_failed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fake-crash");
        let script = "#!/bin/sh\necho 'not json, just a crash log line' >&2\nexit 2\n";
        std::fs::write(&path, script).expect("write fixture plugin");
        let mut perms = std::fs::metadata(&path).expect("stat").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");

        let plugin = PluginConfig {
            plugin_type: "fake-crash".to_string(),
            raw: json!({"type": "fake-crash"}),
        };

        let result = exec_plugin(
            &[dir.path().to_path_buf()],
            &plugin,
            "ADD",
            "/fake/netns",
            "container-1",
            "eth0",
            None,
        )
        .await;

        match result {
            Err(CniError::ProcessFailed {
                plugin,
                exit_code,
                stderr,
            }) => {
                assert_eq!(plugin, "fake-crash");
                assert_eq!(exit_code, Some(2));
                assert!(stderr.contains("crash log line"));
            }
            other => panic!("expected ProcessFailed, got {other:?}"),
        }
    }
}
