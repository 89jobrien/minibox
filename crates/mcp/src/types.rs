//! Tool input and output types for the minibox MCP server.

use crate::error::{McpServerError, Result};
use minibox_core::domain::NetworkMode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Reject empty or whitespace-only identifier fields with a uniform error.
///
/// # Errors
///
/// Returns [`McpServerError::InvalidInput`] when the trimmed value is empty.
pub fn require_non_empty(value: &str, field: &'static str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(McpServerError::InvalidInput(format!(
            "{field} must not be empty"
        )));
    }
    Ok(())
}

/// Parse a user-supplied network mode string into a domain [`NetworkMode`].
///
/// # Errors
///
/// Returns [`McpServerError::InvalidInput`] for unknown network modes.
pub fn parse_network_mode(mode: Option<&str>) -> Result<NetworkMode> {
    match mode.unwrap_or("none") {
        "none" => Ok(NetworkMode::None),
        "bridge" => Ok(NetworkMode::Bridge),
        "host" => Ok(NetworkMode::Host),
        "tailnet" => Ok(NetworkMode::Tailnet),
        other => Err(McpServerError::InvalidInput(format!(
            "unknown network mode: {other}"
        ))),
    }
}

/// Empty input for tools that take no parameters.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct EmptyInput {}

/// Input for daemon connectivity checks.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DoctorInput {}

/// Output for daemon connectivity checks.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DoctorOutput {
    /// Socket path the MCP server attempted to use.
    pub socket_path: String,
    /// Whether a daemon request completed successfully.
    pub connected: bool,
    /// Error text when `connected` is false.
    pub error: Option<String>,
}

/// Bind mount requested for a container run.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct MountInput {
    /// Absolute host path.
    pub host_path: String,
    /// Absolute container path.
    pub container_path: String,
    /// Whether the container mount should be read-only.
    #[serde(default)]
    pub read_only: bool,
}

/// Input for `minibox_run`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct RunContainerInput {
    /// Image name or reference.
    pub image: String,
    /// Optional image tag.
    #[serde(default)]
    pub tag: Option<String>,
    /// Command and arguments to run in the container.
    #[serde(default)]
    pub command: Vec<String>,
    /// Environment variables in `KEY=VALUE` form.
    #[serde(default)]
    pub env: Vec<String>,
    /// Bind mounts.
    #[serde(default)]
    pub mounts: Vec<MountInput>,
    /// Optional memory limit in bytes.
    #[serde(default)]
    pub memory_limit_bytes: Option<u64>,
    /// Optional cgroup CPU weight.
    #[serde(default)]
    pub cpu_weight: Option<u64>,
    /// Network mode: `none`, `bridge`, `host`, or `tailnet`.
    #[serde(default)]
    pub network: Option<String>,
    /// Optional human-readable container name.
    #[serde(default)]
    pub name: Option<String>,
    /// Optional target platform.
    #[serde(default)]
    pub platform: Option<String>,
    /// Whether to auto-remove the container after exit. Defaults to true.
    #[serde(default)]
    pub auto_remove: Option<bool>,
    /// Whether to request privileged mode. Defaults to false.
    #[serde(default)]
    pub privileged: Option<bool>,
}

/// Output for `minibox_run`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunContainerOutput {
    /// Created container ID, if the daemon returned one.
    pub container_id: Option<String>,
    /// Decoded stdout.
    pub stdout: String,
    /// Decoded stderr.
    pub stderr: String,
    /// Exit code from the container process.
    pub exit_code: Option<i32>,
    /// Whether decoded output was truncated.
    pub truncated: bool,
}

/// Input for `minibox_exec`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ExecContainerInput {
    /// Container ID or name. Must be in the Running state.
    pub id: String,
    /// Command and arguments to execute.
    #[serde(default)]
    pub cmd: Vec<String>,
    /// Additional environment variables in `KEY=VALUE` form.
    #[serde(default)]
    pub env: Vec<String>,
    /// Working directory inside the container. Defaults to `/`.
    #[serde(default)]
    pub working_dir: Option<String>,
    /// Run the exec process as a specific user (e.g. `"nobody"`, `"1000:1000"`).
    #[serde(default)]
    pub user: Option<String>,
}

/// Output for `minibox_exec`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExecContainerOutput {
    /// Decoded stdout.
    pub stdout: String,
    /// Decoded stderr.
    pub stderr: String,
    /// Exit code from the exec process.
    pub exit_code: Option<i32>,
    /// Whether decoded output was truncated.
    pub truncated: bool,
}

/// Local schema-facing mirror of `ContainerInfo`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ContainerInfoOutput {
    /// Container ID.
    pub id: String,
    /// Optional human-readable name.
    pub name: Option<String>,
    /// Image name.
    pub image: String,
    /// Command line.
    pub command: String,
    /// Container state.
    pub state: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Container init PID, if known.
    pub pid: Option<u32>,
}

impl From<minibox_core::protocol::ContainerInfo> for ContainerInfoOutput {
    fn from(value: minibox_core::protocol::ContainerInfo) -> Self {
        Self {
            id: value.id,
            name: value.name,
            image: value.image,
            command: value.command,
            state: value.state,
            created_at: value.created_at,
            pid: value.pid,
        }
    }
}

/// Output for `minibox_ps`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PsOutput {
    /// Containers known to the daemon.
    pub containers: Vec<ContainerInfoOutput>,
}

/// Input for `minibox_pull`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PullImageInput {
    /// Image name or reference.
    pub image: String,
    /// Optional image tag.
    #[serde(default)]
    pub tag: Option<String>,
    /// Optional target platform.
    #[serde(default)]
    pub platform: Option<String>,
}

/// Output for `minibox_pull`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PullImageOutput {
    /// Daemon success message.
    pub message: String,
}

/// Input for container logs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct LogsInput {
    /// Container ID or name.
    pub id: String,
}

/// A normalized log entry.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct LogEntry {
    /// Stream name: `stdout` or `stderr`.
    pub stream: String,
    /// Log line.
    pub line: String,
}

/// Output for `minibox_logs`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LogsOutput {
    /// Log lines in daemon order.
    pub lines: Vec<LogEntry>,
}

/// Input for a single container ID operation.
///
/// Emptiness is validated once via [`require_non_empty`] at each tool boundary.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ContainerIdInput {
    /// Container ID or name.
    pub id: String,
}

/// Output for `minibox_images`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImagesOutput {
    /// Cached image references.
    pub images: Vec<String>,
}

/// Output for `minibox_manifest`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ManifestOutput {
    /// Execution manifest JSON.
    pub manifest: Value,
}

/// Output for simple daemon acknowledgements.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SimpleOutput {
    /// Human-readable daemon message.
    pub message: String,
}

/// Input for `minibox_events`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct EventsInput {
    /// How long to collect events before returning, in milliseconds.
    /// Defaults to 2000, clamped to the `1..=10_000` range.
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

/// Local schema-facing mirror of `ContainerEvent`, flattened for MCP output.
/// Exactly one event kind's fields are populated per instance; the rest are `None`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ContainerEventOutput {
    /// Event kind: `created`, `started`, `stopped`, `paused`, `resumed`,
    /// `oom_killed`, `image_pulled`, `image_removed`, or `image_pruned`.
    pub kind: String,
    /// Container ID, for container-scoped event kinds.
    pub id: Option<String>,
    /// Image reference, for `created`/`image_pulled`/`image_removed` events.
    pub image: Option<String>,
    /// Host process ID, for `started` events.
    pub pid: Option<u32>,
    /// Exit code, for `stopped` events.
    pub exit_code: Option<i32>,
    /// Downloaded size in bytes, for `image_pulled` events.
    pub size_bytes: Option<u64>,
    /// Number of images removed, for `image_pruned` events.
    pub count: Option<usize>,
    /// Bytes freed, for `image_pruned` events.
    pub freed_bytes: Option<u64>,
    /// Event time as milliseconds since the Unix epoch.
    pub timestamp_ms: u64,
}

impl From<minibox_core::events::ContainerEvent> for ContainerEventOutput {
    fn from(value: minibox_core::events::ContainerEvent) -> Self {
        fn millis(timestamp: std::time::SystemTime) -> u64 {
            timestamp
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64
        }

        match value {
            minibox_core::events::ContainerEvent::Created {
                id,
                image,
                timestamp,
            } => Self {
                kind: "created".to_string(),
                id: Some(id),
                image: Some(image),
                pid: None,
                exit_code: None,
                size_bytes: None,
                count: None,
                freed_bytes: None,
                timestamp_ms: millis(timestamp),
            },
            minibox_core::events::ContainerEvent::Started { id, pid, timestamp } => Self {
                kind: "started".to_string(),
                id: Some(id),
                image: None,
                pid: Some(pid),
                exit_code: None,
                size_bytes: None,
                count: None,
                freed_bytes: None,
                timestamp_ms: millis(timestamp),
            },
            minibox_core::events::ContainerEvent::Stopped {
                id,
                exit_code,
                timestamp,
            } => Self {
                kind: "stopped".to_string(),
                id: Some(id),
                image: None,
                pid: None,
                exit_code: Some(exit_code),
                size_bytes: None,
                count: None,
                freed_bytes: None,
                timestamp_ms: millis(timestamp),
            },
            minibox_core::events::ContainerEvent::Paused { id, timestamp } => Self {
                kind: "paused".to_string(),
                id: Some(id),
                image: None,
                pid: None,
                exit_code: None,
                size_bytes: None,
                count: None,
                freed_bytes: None,
                timestamp_ms: millis(timestamp),
            },
            minibox_core::events::ContainerEvent::Resumed { id, timestamp } => Self {
                kind: "resumed".to_string(),
                id: Some(id),
                image: None,
                pid: None,
                exit_code: None,
                size_bytes: None,
                count: None,
                freed_bytes: None,
                timestamp_ms: millis(timestamp),
            },
            minibox_core::events::ContainerEvent::OomKilled { id, timestamp } => Self {
                kind: "oom_killed".to_string(),
                id: Some(id),
                image: None,
                pid: None,
                exit_code: None,
                size_bytes: None,
                count: None,
                freed_bytes: None,
                timestamp_ms: millis(timestamp),
            },
            minibox_core::events::ContainerEvent::ImagePulled {
                image,
                size_bytes,
                timestamp,
            } => Self {
                kind: "image_pulled".to_string(),
                id: None,
                image: Some(image),
                pid: None,
                exit_code: None,
                size_bytes: Some(size_bytes),
                count: None,
                freed_bytes: None,
                timestamp_ms: millis(timestamp),
            },
            minibox_core::events::ContainerEvent::ImageRemoved { image, timestamp } => Self {
                kind: "image_removed".to_string(),
                id: None,
                image: Some(image),
                pid: None,
                exit_code: None,
                size_bytes: None,
                count: None,
                freed_bytes: None,
                timestamp_ms: millis(timestamp),
            },
            minibox_core::events::ContainerEvent::ImagePruned {
                count,
                freed_bytes,
                timestamp,
            } => Self {
                kind: "image_pruned".to_string(),
                id: None,
                image: None,
                pid: None,
                exit_code: None,
                size_bytes: None,
                count: Some(count),
                freed_bytes: Some(freed_bytes),
                timestamp_ms: millis(timestamp),
            },
        }
    }
}

/// Output for `minibox_events`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EventsOutput {
    /// Events collected during the window, in daemon order.
    pub events: Vec<ContainerEventOutput>,
    /// Number of events collected.
    pub count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn container_event_output_maps_started_fields() {
        let timestamp = UNIX_EPOCH + std::time::Duration::from_millis(1_000);
        let event = minibox_core::events::ContainerEvent::Started {
            id: "abc123".to_string(),
            pid: 4242,
            timestamp,
        };

        let output = ContainerEventOutput::from(event);

        assert_eq!(output.kind, "started");
        assert_eq!(output.id, Some("abc123".to_string()));
        assert_eq!(output.pid, Some(4242));
        assert_eq!(output.timestamp_ms, 1_000);
        assert_eq!(output.image, None);
        assert_eq!(output.exit_code, None);
    }

    #[test]
    fn container_event_output_maps_image_pruned_fields() {
        let event = minibox_core::events::ContainerEvent::ImagePruned {
            count: 3,
            freed_bytes: 2048,
            timestamp: SystemTime::UNIX_EPOCH,
        };

        let output = ContainerEventOutput::from(event);

        assert_eq!(output.kind, "image_pruned");
        assert_eq!(output.count, Some(3));
        assert_eq!(output.freed_bytes, Some(2048));
        assert_eq!(output.id, None);
    }
}
