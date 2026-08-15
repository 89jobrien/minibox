//! Tool input and output types for the minibox MCP server.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    /// Raw daemon responses.
    pub daemon_responses: Vec<Value>,
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
    /// Raw daemon responses.
    pub daemon_responses: Vec<Value>,
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
    /// Raw daemon responses.
    pub daemon_responses: Vec<Value>,
}

/// Input for a single container ID operation.
// TODO(review): `id` is a bare String with no validation at construction; emptiness is
// checked ad hoc in four separate call sites (containers.rs logs/manifest/simple_id_request).
// A ContainerRef/ContainerId newtype with TryFrom<String> (trim + non-empty) would make
// "empty id" unrepresentable and remove the duplication.
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
    /// Raw daemon responses.
    pub daemon_responses: Vec<Value>,
}
