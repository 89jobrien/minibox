//! [`PluginProbe`] backed by the real filesystem and child processes.

use crate::error::CniError;
use crate::rollout::PluginProbe;
use crate::version::{CNI_COMMAND_VERSION, ENV_CNI_COMMAND, VersionInfo};
use std::path::Path;
use std::process::{Command, Stdio};

/// Stats installed plugin binaries and asks each one for its `VERSION`.
///
/// Synchronous by design: preflight runs once, at rollout time, from a
/// command-line entry point rather than from a daemon hot path.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecPluginProbe;

impl PluginProbe for ExecPluginProbe {
    fn is_installed(&self, candidate: &Path) -> bool {
        candidate.is_file() && is_executable(candidate)
    }

    fn advertised_spec_version(&self, binary: &Path) -> Result<String, CniError> {
        let name = binary.display().to_string();
        let output = Command::new(binary)
            .env(ENV_CNI_COMMAND, CNI_COMMAND_VERSION)
            .stdin(Stdio::null())
            .output()
            .map_err(|err| CniError::RolloutPreflight {
                detail: format!("could not run {name} for CNI_COMMAND=VERSION: {err}"),
            })?;

        if !output.status.success() {
            return Err(CniError::RolloutPreflight {
                detail: format!(
                    "{name} exited with {} for CNI_COMMAND=VERSION: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            });
        }

        let info: VersionInfo =
            serde_json::from_slice(&output.stdout).map_err(|err| CniError::RolloutPreflight {
                detail: format!(
                    "{name} did not answer CNI_COMMAND=VERSION with a decodable reply: {err}"
                ),
            })?;
        Ok(info.cni_version)
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn write_script(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join(name);
        std::fs::write(&path, body).expect("write script");
        let mut perms = std::fs::metadata(&path).expect("stat").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
        path
    }

    fn write_data_file(dir: &Path, name: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "not a program\n").expect("write file");
        path
    }

    #[test]
    fn is_installed_is_false_for_a_missing_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!ExecPluginProbe.is_installed(&dir.path().join("absent")));
    }

    #[test]
    fn is_installed_is_false_for_a_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!ExecPluginProbe.is_installed(dir.path()));
    }

    #[test]
    fn is_installed_is_false_for_a_non_executable_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_data_file(dir.path(), "minibox-bridge");
        assert!(!ExecPluginProbe.is_installed(&path));
    }

    #[test]
    fn is_installed_is_true_for_an_executable_plugin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_script(dir.path(), "minibox-bridge", "#!/bin/sh\nexit 0\n");
        assert!(ExecPluginProbe.is_installed(&path));
    }

    #[test]
    fn advertised_spec_version_reads_a_fixture_plugins_version_reply() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_script(
            dir.path(),
            "fake-bridge",
            &format!(
                "#!/bin/sh\nif [ \"$CNI_COMMAND\" = \"{CNI_COMMAND_VERSION}\" ]; then\n  \
                 echo '{{\"cniVersion\":\"1.0.0\",\"supportedVersions\":[\"1.0.0\"]}}'\nfi\nexit 0\n"
            ),
        );

        let advertised = ExecPluginProbe
            .advertised_spec_version(&path)
            .expect("fixture answers VERSION");
        assert_eq!(advertised, crate::version::CNI_SPEC_VERSION);
    }

    #[test]
    fn advertised_spec_version_rejects_a_plugin_that_exits_non_zero() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_script(
            dir.path(),
            "fake-broken",
            "#!/bin/sh\necho '{\"code\":1,\"msg\":\"nope\"}'\nexit 1\n",
        );
        assert!(matches!(
            ExecPluginProbe.advertised_spec_version(&path),
            Err(CniError::RolloutPreflight { .. })
        ));
    }

    #[test]
    fn advertised_spec_version_rejects_a_plugin_that_prints_nonsense() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_script(dir.path(), "fake-garbage", "#!/bin/sh\necho 'not json'\n");
        let result = ExecPluginProbe.advertised_spec_version(&path);
        match result {
            Err(CniError::RolloutPreflight { detail }) => {
                assert!(detail.contains("fake-garbage"), "{detail}");
            }
            other => panic!("expected RolloutPreflight, got {other:?}"),
        }
    }

    #[test]
    fn advertised_spec_version_rejects_a_plugin_that_answers_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_script(dir.path(), "fake-silent", "#!/bin/sh\nexit 0\n");
        let result = ExecPluginProbe.advertised_spec_version(&path);
        match result {
            Err(CniError::RolloutPreflight { detail }) => {
                assert!(detail.contains("fake-silent"), "{detail}");
            }
            other => panic!("expected RolloutPreflight, got {other:?}"),
        }
    }
}
