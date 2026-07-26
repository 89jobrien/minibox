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
}
