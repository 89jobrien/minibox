//! YAML-driven lifecycle hooks with `inject_as` context injection.
//!
//! A [`Hook`] is a shell command that runs at a named lifecycle point. Hooks
//! can inject their stdout into the agent context under a named key via
//! `inject_as`. The [`HookRunner`] loads hooks from YAML and executes them,
//! returning a map of injected keys to values.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Error from hook execution.
#[derive(Debug, Error)]
pub enum HookError {
    /// Hook command failed with a non-zero exit code.
    #[error("hook '{name}' failed: {message}")]
    ExecutionFailed { name: String, message: String },
    /// YAML deserialization failed.
    #[error("failed to parse hooks YAML: {0}")]
    ParseError(String),
}

/// A single hook definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hook {
    /// Unique name used for logging and injection keys.
    pub name: String,
    /// Shell command to run.
    pub command: String,
    /// If set, the hook's stdout is injected into the agent context under this key.
    pub inject_as: Option<String>,
}

/// A collection of hooks deserialized from YAML.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HookConfig {
    #[serde(default)]
    pub hooks: Vec<Hook>,
}

impl HookConfig {
    /// Parse from a YAML string.
    pub fn from_yaml(yaml: &str) -> Result<Self, HookError> {
        serde_yaml::from_str(yaml).map_err(|e| HookError::ParseError(e.to_string()))
    }
}

/// Runs hooks and collects injected context values.
#[derive(Default)]
pub struct HookRunner {
    config: HookConfig,
}

impl HookRunner {
    /// Create a runner from an existing [`HookConfig`].
    pub fn new(config: HookConfig) -> Self {
        Self { config }
    }

    /// Run all hooks. Returns a map of `inject_as` key → stdout content for
    /// any hook that has `inject_as` set and exits successfully.
    ///
    /// Hook failures are returned as [`HookError`]; the first failure aborts
    /// the run and returns immediately.
    pub fn run_all(&self) -> Result<HashMap<String, String>, HookError> {
        let mut injected = HashMap::new();

        for hook in &self.config.hooks {
            let output = std::process::Command::new("sh")
                .args(["-c", &hook.command])
                .output()
                .map_err(|e| HookError::ExecutionFailed {
                    name: hook.name.clone(),
                    message: e.to_string(),
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                return Err(HookError::ExecutionFailed {
                    name: hook.name.clone(),
                    message: stderr,
                });
            }

            if let Some(key) = &hook.inject_as {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                injected.insert(key.clone(), stdout);
            }
        }

        Ok(injected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
hooks:
  - name: env_info
    command: "echo injected_value"
    inject_as: env_context
  - name: no_inject
    command: "echo ignored"
"#;

    #[test]
    fn hook_config_parses_from_yaml() {
        let cfg = HookConfig::from_yaml(YAML).expect("parse");
        assert_eq!(cfg.hooks.len(), 2);
        assert_eq!(cfg.hooks[0].name, "env_info");
        assert_eq!(cfg.hooks[0].inject_as.as_deref(), Some("env_context"));
        assert!(cfg.hooks[1].inject_as.is_none());
    }

    #[test]
    fn hook_runner_injects_stdout_under_key() {
        let cfg = HookConfig::from_yaml(YAML).expect("parse");
        let runner = HookRunner::new(cfg);
        let injected = runner.run_all().expect("run_all");

        assert!(
            injected.contains_key("env_context"),
            "key should be present"
        );
        assert_eq!(injected["env_context"], "injected_value");
        // no_inject hook has no inject_as — should not appear
        assert!(!injected.contains_key("no_inject"));
    }

    #[test]
    fn hook_runner_returns_error_on_nonzero_exit() {
        let yaml = r#"
hooks:
  - name: failing_hook
    command: "exit 1"
"#;
        let cfg = HookConfig::from_yaml(yaml).expect("parse");
        let runner = HookRunner::new(cfg);
        let result = runner.run_all();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, HookError::ExecutionFailed { .. }));
    }

    #[test]
    fn empty_config_produces_empty_map() {
        let runner = HookRunner::default();
        let injected = runner.run_all().expect("run_all on empty");
        assert!(injected.is_empty());
    }
}
