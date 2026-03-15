//! Daemon <-> CLI communication protocol.
//!
//! Messages are newline-delimited JSON sent over a Unix domain socket at
//! `/run/minibox/miniboxd.sock`. Each message is a single JSON object
//! terminated by `\n`.
//!
//! The `#[serde(tag = "type")]` attribute makes the discriminant field
//! (`"type"`) appear explicitly in the JSON, e.g.:
//!
//! ```json
//! {"type":"Run","image":"ubuntu","tag":"22.04","command":["bash"]}
//! ```

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Requests (CLI -> Daemon)
// ---------------------------------------------------------------------------

/// A request sent from the CLI to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonRequest {
    /// Run a container from the given image.
    Run {
        /// Image name, e.g. `"library/ubuntu"`.
        image: String,
        /// Image tag. Defaults to `"latest"` when `None`.
        tag: Option<String>,
        /// Command and arguments to run inside the container.
        command: Vec<String>,
        /// Optional memory limit in bytes (cgroup `memory.max`).
        memory_limit_bytes: Option<u64>,
        /// Optional CPU weight (cgroup `cpu.weight`, range 1-10000).
        cpu_weight: Option<u64>,
    },

    /// Stop a running container by ID.
    Stop {
        /// Container ID (short UUID).
        id: String,
    },

    /// Remove a stopped container and its resources.
    Remove {
        /// Container ID.
        id: String,
    },

    /// List all containers known to the daemon.
    List,

    /// Pull an image from Docker Hub without running a container.
    Pull {
        /// Image name, e.g. `"library/nginx"`.
        image: String,
        /// Image tag. Defaults to `"latest"` when `None`.
        tag: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Responses (Daemon -> CLI)
// ---------------------------------------------------------------------------

/// A response sent from the daemon back to the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonResponse {
    /// A new container was created (and started).
    ContainerCreated {
        /// The newly assigned container ID.
        id: String,
    },

    /// A generic success acknowledgement.
    Success {
        /// Human-readable status message.
        message: String,
    },

    /// Response to a [`DaemonRequest::List`] request.
    ContainerList {
        /// All containers known to the daemon.
        containers: Vec<ContainerInfo>,
    },

    /// An error occurred processing the request.
    Error {
        /// Human-readable error description.
        message: String,
    },
}

// ---------------------------------------------------------------------------
// Shared data types
// ---------------------------------------------------------------------------

/// Serialisable snapshot of a container's state, used in list responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
    /// Short UUID identifying the container.
    pub id: String,
    /// Image name.
    pub image: String,
    /// Full command line as a single space-separated string.
    pub command: String,
    /// Human-readable state: `"created"`, `"running"`, `"stopped"`, or
    /// `"removed"`.
    pub state: String,
    /// ISO 8601 timestamp at which the container was created.
    pub created_at: String,
    /// PID of the container init process (`None` if not yet started or already
    /// reaped).
    pub pid: Option<u32>,
}

// ---------------------------------------------------------------------------
// Framing helpers
// ---------------------------------------------------------------------------

/// Encode a [`DaemonRequest`] as a newline-terminated JSON frame.
pub fn encode_request(req: &DaemonRequest) -> anyhow::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(req)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Encode a [`DaemonResponse`] as a newline-terminated JSON frame.
pub fn encode_response(resp: &DaemonResponse) -> anyhow::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(resp)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Decode a [`DaemonRequest`] from a line (may or may not include the trailing
/// `\n`).
pub fn decode_request(line: &[u8]) -> anyhow::Result<DaemonRequest> {
    let trimmed = line.strip_suffix(b"\n").unwrap_or(line);
    Ok(serde_json::from_slice(trimmed)?)
}

/// Decode a [`DaemonResponse`] from a line.
pub fn decode_response(line: &[u8]) -> anyhow::Result<DaemonResponse> {
    let trimmed = line.strip_suffix(b"\n").unwrap_or(line);
    Ok(serde_json::from_slice(trimmed)?)
}

// ---------------------------------------------------------------------------
// Socket path constant
// ---------------------------------------------------------------------------

/// Default Unix socket path for the minibox daemon.
pub const DAEMON_SOCKET_PATH: &str = "/run/minibox/miniboxd.sock";
