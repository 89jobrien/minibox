//! Agent safety policy for minibox MCP tools.

use crate::error::{McpServerError, Result};
use crate::types::{RunContainerInput, parse_network_mode, require_non_empty};
use minibox_core::domain::NetworkMode;

/// Default cap on collected daemon response bytes, shared with the client's
/// unconfigured [`crate::client::MiniboxDaemonClient::call`] path.
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// Runtime policy applied before MCP tools call the daemon.
#[derive(Clone, Debug)]
pub struct AgentPolicy {
    /// Agent permissions enabled by environment configuration.
    permissions: Vec<AgentPermission>,
    /// Default memory limit for run requests that omit one.
    pub default_memory_limit_bytes: Option<u64>,
    /// Default CPU weight for run requests that omit one.
    pub default_cpu_weight: Option<u64>,
    /// Maximum collected daemon response bytes.
    pub max_output_bytes: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentPermission {
    MutatingTools,
    Privileged,
    BindMounts,
    HostNetwork,
}

impl AgentPolicy {
    /// Conservative defaults for agent-controlled execution.
    #[must_use]
    pub const fn safe_default() -> Self {
        Self {
            permissions: Vec::new(),
            default_memory_limit_bytes: Some(512 * 1024 * 1024),
            default_cpu_weight: Some(100),
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }

    /// Build policy from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        let mut policy = Self::safe_default();
        policy.enable_from_env("MINIBOX_MCP_ALLOW_MUTATION", AgentPermission::MutatingTools);
        policy.enable_from_env("MINIBOX_MCP_ALLOW_BIND_MOUNTS", AgentPermission::BindMounts);
        policy.enable_from_env("MINIBOX_MCP_ALLOW_PRIVILEGED", AgentPermission::Privileged);
        policy.enable_from_env(
            "MINIBOX_MCP_ALLOW_HOST_NETWORK",
            AgentPermission::HostNetwork,
        );
        if let Ok(value) = std::env::var("MINIBOX_MCP_MAX_OUTPUT_BYTES") {
            match value.parse::<usize>() {
                Ok(parsed) => policy.max_output_bytes = parsed,
                Err(error) => tracing::warn!(
                    value = %value,
                    error = %error,
                    "policy: malformed MINIBOX_MCP_MAX_OUTPUT_BYTES; using default"
                ),
            }
        }
        policy
    }

    /// Validate a safe run request.
    ///
    /// # Errors
    ///
    /// Returns a policy denial or invalid-input error for unsafe run options.
    pub fn validate_run(&self, input: &RunContainerInput) -> Result<()> {
        if input.privileged.unwrap_or(false) && !self.allows(AgentPermission::Privileged) {
            return Err(McpServerError::PolicyDenied {
                tool: "minibox_run",
                reason: "privileged runs require MINIBOX_MCP_ALLOW_PRIVILEGED=true".to_string(),
            });
        }
        if !input.mounts.is_empty() && !self.allows(AgentPermission::BindMounts) {
            return Err(McpServerError::PolicyDenied {
                tool: "minibox_run",
                reason: "bind mounts require MINIBOX_MCP_ALLOW_BIND_MOUNTS=true".to_string(),
            });
        }
        // Parse before gating so an invalid or aliased network value cannot
        // slip past the host-network check.
        let network_mode = parse_network_mode(input.network.as_deref())?;
        if network_mode == NetworkMode::Host && !self.allows(AgentPermission::HostNetwork) {
            return Err(McpServerError::PolicyDenied {
                tool: "minibox_run",
                reason: "host networking requires MINIBOX_MCP_ALLOW_HOST_NETWORK=true".to_string(),
            });
        }
        require_non_empty(&input.image, "image")?;
        Ok(())
    }

    /// Validate a lifecycle mutation tool.
    ///
    /// # Errors
    ///
    /// Returns a policy denial when lifecycle mutation is disabled.
    pub fn validate_mutation(&self, tool_name: &'static str) -> Result<()> {
        if self.allows(AgentPermission::MutatingTools) {
            Ok(())
        } else {
            Err(McpServerError::PolicyDenied {
                tool: tool_name,
                reason: "set MINIBOX_MCP_ALLOW_MUTATION=true to enable this tool".to_string(),
            })
        }
    }

    fn enable_from_env(&mut self, name: &str, permission: AgentPermission) {
        if env_bool(name) {
            self.permissions.push(permission);
        }
    }

    fn allows(&self, permission: AgentPermission) -> bool {
        self.permissions.contains(&permission)
    }
    // TODO(review): enforcement is a plain runtime bool check callers can simply omit —
    // pull_image (images.rs) does. Consider returning a marker type (e.g. Authorized<T>)
    // from validate_run/validate_mutation that daemon-call functions require, so a future
    // mutating tool can't compile without passing the gate.
}

fn env_bool(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| {
        matches!(
            value.as_str(),
            "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MountInput, RunContainerInput};
    use std::sync::Mutex;

    /// Serializes env-mutating tests; `set_var`/`remove_var` are unsafe in
    /// edition 2024 and race with concurrent reads across parallel tests.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvVarGuard {
        name: &'static str,
    }

    impl EnvVarGuard {
        fn set(name: &'static str, value: &str) -> Self {
            // SAFETY: all mutation of these process-wide env vars happens
            // under ENV_LOCK, so no other thread reads or writes them
            // concurrently for the guard's lifetime.
            unsafe { std::env::set_var(name, value) };
            Self { name }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // SAFETY: still under the ENV_LOCK held by the owning test.
            unsafe { std::env::remove_var(self.name) };
        }
    }

    fn run_input() -> RunContainerInput {
        RunContainerInput {
            image: "alpine".to_string(),
            ..RunContainerInput::default()
        }
    }

    #[test]
    fn safe_default_denies_privileged_run() {
        let policy = AgentPolicy::safe_default();
        let input = RunContainerInput {
            privileged: Some(true),
            ..run_input()
        };

        assert!(matches!(
            policy.validate_run(&input),
            Err(McpServerError::PolicyDenied {
                tool: "minibox_run",
                ..
            })
        ));
    }

    #[test]
    fn safe_default_denies_bind_mounts() {
        let policy = AgentPolicy::safe_default();
        let input = RunContainerInput {
            mounts: vec![MountInput {
                host_path: "/tmp".to_string(),
                container_path: "/host".to_string(),
                read_only: true,
            }],
            ..run_input()
        };

        assert!(policy.validate_run(&input).is_err());
    }

    #[test]
    fn safe_default_denies_stop_rm_mutations() {
        let policy = AgentPolicy::safe_default();

        assert!(policy.validate_mutation("minibox_rm").is_err());
    }

    #[test]
    fn validate_run_rejects_unknown_network_mode() {
        let policy = AgentPolicy::safe_default();
        let input = RunContainerInput {
            network: Some("hostt".to_string()),
            ..run_input()
        };

        assert!(matches!(
            policy.validate_run(&input),
            Err(McpServerError::InvalidInput(_))
        ));
    }

    #[test]
    fn from_env_defaults_deny_all_permissions() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let policy = AgentPolicy::from_env();

        assert!(policy.validate_mutation("minibox_rm").is_err());
        let input = RunContainerInput {
            privileged: Some(true),
            ..run_input()
        };
        assert!(policy.validate_run(&input).is_err());
    }

    #[test]
    fn from_env_enables_mutation_permission() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let _guard = EnvVarGuard::set("MINIBOX_MCP_ALLOW_MUTATION", "true");
        let policy = AgentPolicy::from_env();

        assert!(policy.validate_mutation("minibox_rm").is_ok());
    }

    #[test]
    fn from_env_enables_privileged_and_host_network() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let _privileged = EnvVarGuard::set("MINIBOX_MCP_ALLOW_PRIVILEGED", "1");
        let _host = EnvVarGuard::set("MINIBOX_MCP_ALLOW_HOST_NETWORK", "on");
        let policy = AgentPolicy::from_env();
        let input = RunContainerInput {
            privileged: Some(true),
            network: Some("host".to_string()),
            ..run_input()
        };

        assert!(policy.validate_run(&input).is_ok());
    }

    #[test]
    fn from_env_reads_max_output_bytes() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let _guard = EnvVarGuard::set("MINIBOX_MCP_MAX_OUTPUT_BYTES", "2048");
        let policy = AgentPolicy::from_env();

        assert_eq!(policy.max_output_bytes, 2048);
    }

    #[test]
    fn from_env_keeps_default_on_malformed_max_output_bytes() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let _guard = EnvVarGuard::set("MINIBOX_MCP_MAX_OUTPUT_BYTES", "not-a-number");
        let policy = AgentPolicy::from_env();

        assert_eq!(policy.max_output_bytes, DEFAULT_MAX_OUTPUT_BYTES);
    }
}
