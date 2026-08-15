//! Agent safety policy for minibox MCP tools.

use crate::error::{McpServerError, Result};
use crate::types::RunContainerInput;

const DEFAULT_MAX_OUTPUT_BYTES: usize = 1024 * 1024;

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
        // TODO(review): parse failure here is silently swallowed — an operator's typo'd
        // or malformed MINIBOX_MCP_MAX_OUTPUT_BYTES falls back to the 1 MiB default with
        // no log line anywhere in this crate. Add tracing::warn!(value, ...) on parse
        // failure, or make from_env() fallible and fail loudly at startup.
        if let Ok(value) = std::env::var("MINIBOX_MCP_MAX_OUTPUT_BYTES")
            && let Ok(parsed) = value.parse::<usize>()
        {
            policy.max_output_bytes = parsed;
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
        // TODO(review): compares the raw input string rather than a parsed NetworkMode;
        // validation of the network string happens later in containers::parse_network_mode,
        // after this decision is made. Parse before gating so an invalid/aliased value
        // can't slip past the host-network check.
        if input.network.as_deref() == Some("host") && !self.allows(AgentPermission::HostNetwork) {
            return Err(McpServerError::PolicyDenied {
                tool: "minibox_run",
                reason: "host networking requires MINIBOX_MCP_ALLOW_HOST_NETWORK=true".to_string(),
            });
        }
        if input.image.trim().is_empty() {
            return Err(McpServerError::InvalidInput(
                "image must not be empty".to_string(),
            ));
        }
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
// TODO(review): all tests below exercise safe_default() (deny path) only. from_env() —
// the real binary's boot path — has zero coverage: a typo in an env var name or a
// regressed match arm in env_bool would silently misconfigure agent permissions with no
// test catching it. Add tests that set MINIBOX_MCP_ALLOW_* (guarded by a static
// Mutex<()> per this repo's env-mutation convention, since set_var is unsafe in edition
// 2024) and assert from_env() actually flips validate_run/validate_mutation to Ok.
mod tests {
    use super::*;
    use crate::types::{MountInput, RunContainerInput};

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
}
