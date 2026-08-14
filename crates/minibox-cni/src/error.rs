//! Errors from CNI plugin execution.

use std::path::PathBuf;

/// Errors from CNI plugin execution.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum CniError {
    /// A plugin type was not a single filename component.
    #[error("CNI plugin type '{plugin}' must be a single normal path component")]
    #[diagnostic(code(minibox::cni::invalid_plugin_type))]
    InvalidPluginType {
        /// The invalid plugin type supplied by the CNI configuration.
        plugin: String,
    },

    /// A required plugin binary was not found on the configured `CNI_PATH`.
    #[error("CNI plugin '{plugin}' not found on CNI_PATH")]
    #[diagnostic(code(minibox::cni::plugin_not_found))]
    PluginNotFound {
        /// The plugin type that was requested (e.g. `"bridge"`).
        plugin: String,
        /// The directories that were searched.
        searched: Vec<PathBuf>,
    },

    /// A plugin returned a CNI-spec structured error object on stdout.
    #[error("CNI plugin '{plugin}' failed: {msg}")]
    #[diagnostic(code(minibox::cni::plugin_error))]
    PluginError {
        /// The plugin type that failed.
        plugin: String,
        /// The CNI spec error code, if the plugin reported one.
        code: Option<u32>,
        /// Human-readable error message from the plugin.
        msg: String,
        /// Optional additional detail from the plugin.
        details: Option<String>,
    },

    /// A plugin process exited non-zero (or was signal-killed) without a
    /// structured CNI error object on stdout.
    #[error("CNI plugin '{plugin}' exited with status {exit_code:?}")]
    #[diagnostic(code(minibox::cni::process_failed))]
    ProcessFailed {
        /// The plugin type that failed.
        plugin: String,
        /// Process exit code, or `None` if signal-killed.
        exit_code: Option<i32>,
        /// Captured stderr output.
        stderr: String,
    },

    /// Failed to parse a `.conflist` file or a plugin's JSON payload.
    #[error("failed to parse CNI JSON: {0}")]
    #[diagnostic(code(minibox::cni::config_parse))]
    ConfigParse(#[from] serde_json::Error),

    /// I/O error spawning or communicating with a plugin process.
    #[error("CNI plugin I/O error: {0}")]
    #[diagnostic(code(minibox::cni::io))]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn plugin_not_found_display_names_the_plugin() {
        let err = CniError::PluginNotFound {
            plugin: "bridge".to_string(),
            searched: vec![PathBuf::from("/opt/cni/bin")],
        };
        assert!(err.to_string().contains("bridge"));
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn plugin_error_display_includes_msg() {
        let err = CniError::PluginError {
            plugin: "host-local".to_string(),
            code: Some(7),
            msg: "no IP addresses available".to_string(),
            details: None,
        };
        assert!(err.to_string().contains("host-local"));
        assert!(err.to_string().contains("no IP addresses available"));
    }
}
