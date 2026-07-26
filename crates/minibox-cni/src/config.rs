//! Parsing of CNI `.conflist` network configuration files.

use crate::error::CniError;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Parsed CNI network configuration list (`.conflist` format).
#[derive(Debug, Clone, Deserialize)]
pub struct NetworkConfigList {
    /// CNI spec version this config conforms to.
    #[serde(rename = "cniVersion")]
    pub cni_version: String,
    /// Network name.
    pub name: String,
    /// Ordered chain of plugins to invoke.
    pub plugins: Vec<PluginConfig>,
}

/// A single plugin's configuration within a `.conflist` chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    /// Plugin binary name (e.g. `"bridge"`, `"host-local"`).
    #[serde(rename = "type")]
    pub plugin_type: String,
    /// The plugin's own configuration fields, kept as raw JSON since each
    /// plugin defines its own schema.
    #[serde(flatten)]
    pub raw: serde_json::Value,
}

impl NetworkConfigList {
    /// Parse a `.conflist` file from disk.
    ///
    /// # Errors
    ///
    /// Returns [`CniError::Io`] if the file cannot be read, or
    /// [`CniError::ConfigParse`] if it is not valid CNI JSON.
    pub fn from_file(path: &Path) -> Result<Self, CniError> {
        let bytes = std::fs::read(path)?;
        let parsed = serde_json::from_slice(&bytes)?;
        Ok(parsed)
    }

    /// Run the full ADD chain in plugin order, threading `prevResult`
    /// between plugins. On mid-chain failure, rolls back (`DEL` in
    /// reverse) the already-succeeded plugins before returning the error.
    ///
    /// # Errors
    ///
    /// Returns the first plugin failure encountered, after best-effort
    /// rollback of any plugins that had already succeeded.
    #[tracing::instrument(skip(self), fields(network = %self.name, plugin_count = self.plugins.len()))]
    pub async fn add(
        &self,
        cni_path: &[std::path::PathBuf],
        netns: &str,
        container_id: &str,
        ifname: &str,
    ) -> Result<crate::result::CniResult, CniError> {
        let mut prev_result: Option<serde_json::Value> = None;
        let mut succeeded: Vec<&PluginConfig> = Vec::new();

        for plugin in &self.plugins {
            match crate::exec::exec_plugin(
                cni_path,
                plugin,
                "ADD",
                netns,
                container_id,
                ifname,
                prev_result.as_ref(),
            )
            .await
            {
                Ok(result) => {
                    succeeded.push(plugin);
                    prev_result = Some(result);
                }
                Err(err) => {
                    for rollback_plugin in succeeded.iter().rev() {
                        if let Err(rollback_err) = crate::exec::exec_plugin(
                            cni_path,
                            rollback_plugin,
                            "DEL",
                            netns,
                            container_id,
                            ifname,
                            None,
                        )
                        .await
                        {
                            tracing::warn!(
                                plugin = %rollback_plugin.plugin_type,
                                error = %rollback_err,
                                "cni: rollback DEL failed after mid-chain ADD failure"
                            );
                        }
                    }
                    return Err(err);
                }
            }
        }

        let final_result = prev_result.ok_or_else(|| CniError::PluginError {
            plugin: self.name.clone(),
            code: None,
            msg: "empty plugin chain produced no result".to_string(),
            details: None,
        })?;
        serde_json::from_value(final_result).map_err(CniError::ConfigParse)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_conflist(contents: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().expect("create tempfile");
        file.write_all(contents.as_bytes()).expect("write conflist");
        file
    }

    #[test]
    fn from_file_parses_plugin_chain() {
        let file = write_conflist(
            r#"{
                "cniVersion": "1.0.0",
                "name": "minibox0",
                "plugins": [
                    {"type": "bridge", "bridge": "minibox0"},
                    {"type": "portmap", "capabilities": {"portMappings": true}}
                ]
            }"#,
        );
        let parsed = NetworkConfigList::from_file(file.path()).expect("parse conflist");
        assert_eq!(parsed.cni_version, "1.0.0");
        assert_eq!(parsed.name, "minibox0");
        assert_eq!(parsed.plugins.len(), 2);
        assert_eq!(parsed.plugins[0].plugin_type, "bridge");
        assert_eq!(parsed.plugins[1].plugin_type, "portmap");
    }

    #[test]
    fn from_file_rejects_malformed_json() {
        let file = write_conflist("not json");
        let result = NetworkConfigList::from_file(file.path());
        assert!(matches!(result, Err(CniError::ConfigParse(_))));
    }

    #[test]
    fn from_file_rejects_missing_file() {
        let result = NetworkConfigList::from_file(std::path::Path::new("/nonexistent/x.conflist"));
        assert!(matches!(result, Err(CniError::Io(_))));
    }

    fn write_executable(dir: &std::path::Path, name: &str, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
    }

    #[tokio::test]
    async fn add_threads_prev_result_across_chain() {
        let bin_dir = tempfile::tempdir().expect("bin tempdir");
        // First plugin ignores prevResult (there is none), emits an IP.
        write_executable(
            bin_dir.path(),
            "fake-bridge",
            "#!/bin/sh\necho '{\"cniVersion\":\"1.0.0\",\"ips\":[{\"address\":\"10.0.0.5/24\"}]}'\nexit 0\n",
        );
        // Second plugin reads stdin, asserts prevResult's IP made it through, then re-emits it.
        write_executable(
            bin_dir.path(),
            "fake-portmap",
            "#!/bin/sh\ncat > /tmp/minibox-cni-test-portmap-input.json\n\
             if grep -q '10.0.0.5/24' /tmp/minibox-cni-test-portmap-input.json; then\n\
             echo '{\"cniVersion\":\"1.0.0\",\"ips\":[{\"address\":\"10.0.0.5/24\"}]}'\nexit 0\n\
             else\nexit 1\nfi\n",
        );

        let list = NetworkConfigList {
            cni_version: "1.0.0".to_string(),
            name: "minibox0".to_string(),
            plugins: vec![
                PluginConfig {
                    plugin_type: "fake-bridge".to_string(),
                    raw: serde_json::json!({"type": "fake-bridge"}),
                },
                PluginConfig {
                    plugin_type: "fake-portmap".to_string(),
                    raw: serde_json::json!({"type": "fake-portmap"}),
                },
            ],
        };

        let result = list
            .add(
                &[bin_dir.path().to_path_buf()],
                "/fake/netns",
                "container-1",
                "eth0",
            )
            .await
            .expect("add should succeed");

        assert_eq!(result.ips[0].address, "10.0.0.5/24");
        let _ = std::fs::remove_file("/tmp/minibox-cni-test-portmap-input.json");
    }

    #[tokio::test]
    async fn add_rolls_back_succeeded_plugins_on_mid_chain_failure() {
        let bin_dir = tempfile::tempdir().expect("bin tempdir");
        let del_marker = bin_dir.path().join("bridge-was-deleted");

        // Plugin 1 succeeds on ADD, and on DEL touches a marker file so the
        // test can assert rollback actually ran.
        write_executable(
            bin_dir.path(),
            "fake-bridge",
            &format!(
                "#!/bin/sh\nif [ \"$CNI_COMMAND\" = \"ADD\" ]; then\n  echo '{{\"cniVersion\":\"1.0.0\"}}'\nelif [ \"$CNI_COMMAND\" = \"DEL\" ]; then\n  touch {}\nfi\nexit 0\n",
                del_marker.display()
            ),
        );
        // Plugin 2 always fails ADD with a structured error.
        write_executable(
            bin_dir.path(),
            "fake-portmap",
            "#!/bin/sh\necho '{\"code\":9,\"msg\":\"boom\"}'\nexit 1\n",
        );

        let list = NetworkConfigList {
            cni_version: "1.0.0".to_string(),
            name: "minibox0".to_string(),
            plugins: vec![
                PluginConfig {
                    plugin_type: "fake-bridge".to_string(),
                    raw: serde_json::json!({"type": "fake-bridge"}),
                },
                PluginConfig {
                    plugin_type: "fake-portmap".to_string(),
                    raw: serde_json::json!({"type": "fake-portmap"}),
                },
            ],
        };

        let result = list
            .add(
                &[bin_dir.path().to_path_buf()],
                "/fake/netns",
                "container-1",
                "eth0",
            )
            .await;

        assert!(matches!(
            result,
            Err(CniError::PluginError { code: Some(9), .. })
        ));
        assert!(
            del_marker.exists(),
            "rollback DEL should have run for the succeeded plugin"
        );
    }
}
