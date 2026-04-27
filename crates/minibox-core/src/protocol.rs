//! Daemon <-> CLI communication protocol.
//!
//! Messages are newline-delimited JSON sent over a Unix domain socket at
//! `/run/minibox/miniboxd.sock` ([`DAEMON_SOCKET_PATH`]). Each message is a
//! single JSON object terminated by `\n`.
//!
//! The `#[serde(tag = "type")]` attribute makes the discriminant field
//! (`"type"`) appear explicitly in the JSON, e.g.:
//!
//! ```json
//! {"type":"Run","image":"ubuntu","tag":"22.04","command":["bash"]}
//! ```
//!
//! # Message flow
//!
//! **Non-ephemeral (fire-and-forget) run:**
//! ```text
//! CLI  ──Run{ephemeral:false}──►  Daemon
//! CLI  ◄──ContainerCreated{id}──  Daemon
//! ```
//! The CLI exits immediately after receiving `ContainerCreated`. The container
//! continues running in the background and appears in `List` responses.
//!
//! **Ephemeral (streaming) run** (`minibox run` default):
//! ```text
//! CLI  ──Run{ephemeral:true}──►  Daemon
//! CLI  ◄──ContainerCreated{id}── Daemon   (container ID assigned)
//! CLI  ◄──ContainerOutput{..}──  Daemon   (zero or more stdout/stderr chunks)
//! CLI  ◄──ContainerStopped{..}── Daemon   (exactly once; carries exit code)
//! ```
//! The CLI prints each `ContainerOutput` chunk to the terminal in real time and
//! exits with the container's exit code from `ContainerStopped`.
//!
//! # Framing
//!
//! Use [`encode_request`] / [`decode_request`] and [`encode_response`] /
//! [`decode_response`] to serialize and deserialize messages. These helpers
//! append (or strip) the trailing `\n` framing byte.

use crate::domain::{BindMount, NetworkMode, SessionId};
use serde::{Deserialize, Serialize};

/// Serializable registry credentials for protocol transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PushCredentials {
    Anonymous,
    Basic { username: String, password: String },
    Token { token: String },
}

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
        /// If `true`, the daemon streams stdout/stderr back and sends
        /// `ContainerStopped` when the process exits.  Defaults to `false`
        /// (fire-and-forget behaviour) for backwards compatibility.
        #[serde(default)]
        ephemeral: bool,
        /// Network mode for the container.
        ///
        /// `None` maps to `NetworkConfig::default()`, which selects
        /// `NetworkMode::None` (isolated namespace, no network connectivity).
        #[serde(default)]
        network: Option<NetworkMode>,
        /// Environment variables to set inside the container, in `KEY=VALUE` form.
        ///
        /// These are merged with the container's default environment (PATH, TERM).
        /// User-supplied values take precedence over defaults for duplicate keys.
        #[serde(default)]
        env: Vec<String>,
        /// Bind mounts to apply inside the container.
        ///
        /// Each entry is mounted before `pivot_root` in the container's mount namespace.
        /// On the Colima adapter, host paths must be under `$HOME` or `/tmp`.
        #[serde(default)]
        mounts: Vec<BindMount>,
        /// If `true`, the container process runs with a full Linux capability set.
        ///
        /// Required for Docker-in-Docker (DinD) use cases where the inner process
        /// needs `CAP_SYS_ADMIN`, `CAP_NET_ADMIN`, etc. to create namespaces.
        #[serde(default)]
        privileged: bool,
        /// Optional human-readable name for the container.
        ///
        /// When set, the container can be referenced by name in `stop` and `rm`
        /// commands in addition to its auto-generated ID.  Names must be unique;
        /// if a container with the same name already exists the request fails.
        #[serde(default)]
        name: Option<String>,
        /// If `true`, allocate a PTY and stream stdin/stdout as a terminal session.
        #[serde(default)]
        tty: bool,
        /// Override the image's default entrypoint.
        ///
        /// When set, this replaces the image's `ENTRYPOINT` directive.
        /// `command` becomes the arguments to this entrypoint.
        #[serde(default)]
        entrypoint: Option<String>,
        /// Run the container process as a specific user (e.g. `"nobody"`, `"1000:1000"`).
        ///
        /// Maps to the `--user` / `-u` Docker flag. When `None`, the container
        /// runs as root (or the image's default `USER` directive).
        #[serde(default)]
        user: Option<String>,
        /// Automatically remove the container when it exits.
        #[serde(default)]
        auto_remove: bool,
        /// Scheduling priority for this container run.
        #[serde(default)]
        priority: Option<slashcrux::Priority>,
        /// Urgency hint for the scheduler.
        #[serde(default)]
        urgency: Option<slashcrux::Urgency>,
        /// Agentic execution context — workflow variables and bindings.
        #[serde(default)]
        execution_context: Option<slashcrux::ExecutionContext>,
    },

    /// Stop a running container by ID.
    Stop {
        /// Container ID (short UUID).
        id: String,
    },

    /// Freeze all processes in a running container via cgroup.freeze.
    PauseContainer {
        /// Container ID to pause.
        id: String,
    },

    /// Thaw a paused container.
    ResumeContainer {
        /// Container ID to resume.
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

    /// Load a local OCI image tarball into the daemon's image store.
    LoadImage {
        /// Absolute path to the OCI tarball on the host filesystem.
        path: String,
        /// Image name to register (e.g. `"minibox-tester"`).
        name: String,
        /// Image tag to register (e.g. `"latest"`).
        tag: String,
    },

    /// Execute a command inside an already-running container.
    Exec {
        /// ID of the target container (must be in Running state).
        container_id: String,
        /// Command and arguments to execute.
        cmd: Vec<String>,
        /// Additional environment variables in `KEY=VALUE` form.
        #[serde(default)]
        env: Vec<String>,
        /// Working directory inside the container. Defaults to `/`.
        #[serde(default)]
        working_dir: Option<String>,
        /// If `true`, allocate a pseudo-TTY for the exec process.
        #[serde(default)]
        tty: bool,
        /// Run the exec process as a specific user (e.g. `"nobody"`, `"1000:1000"`).
        #[serde(default)]
        user: Option<String>,
    },

    /// Send raw bytes to a running exec or run session stdin (base64-encoded).
    SendInput { session_id: SessionId, data: String },

    /// Notify the daemon the client terminal was resized.
    ResizePty {
        session_id: SessionId,
        cols: u16,
        rows: u16,
    },

    /// Push a locally-stored image to a remote OCI registry.
    Push {
        /// Image reference to push (e.g. `"docker.io/library/ubuntu:22.04"`).
        image_ref: String,
        /// Credentials for authenticating to the target registry.
        credentials: PushCredentials,
    },

    /// Snapshot a container's filesystem changes into a new local image.
    Commit {
        container_id: String,
        target_image: String,
        #[serde(default)]
        author: Option<String>,
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        env_overrides: Vec<String>,
        #[serde(default)]
        cmd_override: Option<Vec<String>>,
    },

    /// Build an image from a Dockerfile.
    Build {
        /// Dockerfile content (as string).
        dockerfile: String,
        /// Build context directory path on the daemon host.
        context_path: String,
        /// Target tag for the built image.
        tag: String,
        /// Build-time argument overrides.
        #[serde(default)]
        build_args: Vec<(String, String)>,
        /// When `true`, skip any cached layers.
        #[serde(default)]
        no_cache: bool,
    },

    /// Subscribe to the container event stream.
    ///
    /// The daemon will send `Event` responses until the connection closes.
    SubscribeEvents,

    /// Remove unused images (optionally dry-run).
    Prune {
        #[serde(default)]
        dry_run: bool,
    },

    /// Remove a specific image by reference.
    RemoveImage {
        /// Image reference, e.g. `"alpine:latest"`.
        image_ref: String,
    },

    /// Retrieve stored log output for a container.
    ///
    /// The daemon will send zero or more `LogLine` responses followed by
    /// `Success { message: "end of log" }` (when `follow: false`).
    ContainerLogs {
        /// Container ID or name to retrieve logs for.
        container_id: String,
        /// If `true`, keep the connection open and stream new output as it
        /// arrives (not yet implemented — reserved for future use).
        #[serde(default)]
        follow: bool,
    },

    /// Run a crux pipeline inside a container.
    ///
    /// Higher-level than `Run` — bundles image pull + container create +
    /// pipeline execution + trace collection.
    RunPipeline {
        /// Path to the `.cruxx` pipeline file (host-side).
        pipeline_path: String,
        /// Optional JSON input to the pipeline.
        #[serde(default)]
        input: Option<serde_json::Value>,
        /// Container image to use. Defaults to `cruxx-runtime:latest`.
        #[serde(default)]
        image: Option<String>,
        /// Token/step/time budget for the pipeline execution.
        #[serde(default)]
        budget: Option<serde_json::Value>,
        /// Additional environment variables as (KEY, VALUE) pairs.
        #[serde(default)]
        env: Vec<(String, String)>,
        /// Maximum container nesting depth (daemon-enforced).
        /// Defaults to 3. Requests exceeding this are rejected.
        #[serde(default = "default_max_depth")]
        max_depth: u32,
        /// Scheduling priority for this pipeline run.
        #[serde(default)]
        priority: Option<slashcrux::Priority>,
        /// Urgency hint for the scheduler.
        #[serde(default)]
        urgency: Option<slashcrux::Urgency>,
        /// Agentic execution context — workflow variables and bindings.
        #[serde(default)]
        execution_context: Option<slashcrux::ExecutionContext>,
    },

    /// Save a VM state snapshot for a container.
    SaveSnapshot {
        /// Container ID to snapshot.
        id: String,
        /// Optional human-readable snapshot name (auto-generated if omitted).
        #[serde(default)]
        name: Option<String>,
    },

    /// Restore a VM state snapshot for a container.
    RestoreSnapshot {
        /// Container ID to restore.
        id: String,
        /// Snapshot name to restore.
        name: String,
    },

    /// List available snapshots for a container.
    ListSnapshots {
        /// Container ID.
        id: String,
    },
}

fn default_max_depth() -> u32 {
    3
}

// ---------------------------------------------------------------------------
// Responses (Daemon -> CLI)
// ---------------------------------------------------------------------------

/// Which output stream a [`DaemonResponse::ContainerOutput`] chunk came from.
///
/// Serialized as lowercase strings (`"stdout"` / `"stderr"`) via
/// `#[serde(rename_all = "lowercase")]`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OutputStreamKind {
    /// Data came from the container's standard output (file descriptor 1).
    Stdout,
    /// Data came from the container's standard error (file descriptor 2).
    Stderr,
}

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

    /// Confirmation that a container was paused.
    ContainerPaused {
        /// The container ID.
        id: String,
    },

    /// Confirmation that a container was resumed.
    ContainerResumed {
        /// The container ID.
        id: String,
    },

    /// Response to a [`DaemonRequest::List`] request.
    ContainerList {
        /// All containers known to the daemon.
        containers: Vec<ContainerInfo>,
    },

    /// Confirmation that a local image tarball was loaded successfully.
    ImageLoaded {
        /// The image reference that was registered, e.g. `"minibox-tester:latest"`.
        image: String,
    },

    /// An error occurred processing the request.
    Error {
        /// Human-readable error description.
        message: String,
    },

    /// A chunk of output from a running container's stdout or stderr.
    ///
    /// `data` is base64-encoded raw bytes.  Sent zero or more times before
    /// [`DaemonResponse::ContainerStopped`].  Only emitted when the
    /// originating [`DaemonRequest::Run`] had `ephemeral: true`.
    ContainerOutput {
        /// Which stream the data came from.
        stream: OutputStreamKind,
        /// Base64-encoded raw bytes.
        data: String,
    },

    /// Terminal message after a streaming run.
    ///
    /// Exactly one per ephemeral run; signals end of the
    /// [`DaemonResponse::ContainerOutput`] stream.
    ContainerStopped {
        /// Exit code of the container process.
        exit_code: i32,
    },

    /// Sent once after exec setup completes, before any output arrives.
    ///
    /// Non-terminal: output chunks and a final `ContainerStopped` follow.
    ExecStarted {
        /// Unique identifier for this exec instance.
        exec_id: String,
    },

    /// Push progress update for a single layer.
    ///
    /// Non-terminal: sent zero or more times during a push operation before
    /// the final `Success` or `Error`.
    PushProgress {
        /// Digest of the layer being uploaded.
        layer_digest: String,
        /// Bytes uploaded so far for this layer.
        bytes_uploaded: u64,
        /// Total bytes in this layer.
        total_bytes: u64,
    },

    /// Streaming build log line.
    ///
    /// Non-terminal: sent once per Dockerfile step before `BuildComplete`.
    BuildOutput {
        /// 1-based step index.
        step: u32,
        /// Total steps in the Dockerfile.
        total_steps: u32,
        /// Human-readable step description.
        message: String,
    },

    /// Build completed successfully.
    BuildComplete {
        /// Content-addressable ID of the new image.
        image_id: String,
        /// Tag applied to the built image.
        tag: String,
    },

    /// A container lifecycle event.
    ///
    /// Non-terminal: sent zero or more times until the connection closes.
    /// Emitted in response to [`DaemonRequest::SubscribeEvents`].
    Event {
        /// The container lifecycle event payload.
        event: crate::events::ContainerEvent,
    },

    /// Result of a prune operation.
    Pruned {
        /// Image refs that were (or would be) removed.
        removed: Vec<String>,
        /// Bytes freed (or that would be freed in dry-run mode).
        freed_bytes: u64,
        /// True if this was a dry run.
        dry_run: bool,
    },

    /// A single line of stored log output.
    ///
    /// Non-terminal: sent zero or more times before the terminal `Success`
    /// (or `Error`) response from a [`DaemonRequest::ContainerLogs`] request.
    LogLine {
        /// Which stream the line originated from.
        stream: OutputStreamKind,
        /// The log line content (without trailing newline).
        line: String,
    },

    /// Pipeline execution completed.
    ///
    /// Terminal response for `RunPipeline` requests. The `trace` field
    /// contains the full execution trace serialized as JSON — consumers
    /// deserialize into their concrete trace type.
    PipelineComplete {
        /// Serialized execution trace (crux-agnostic JSON).
        trace: serde_json::Value,
        /// Container ID that ran the pipeline.
        container_id: String,
        /// Exit code of the `crux run` process.
        exit_code: i32,
    },

    /// Confirmation that a snapshot was saved.
    SnapshotSaved {
        /// Snapshot metadata.
        info: crate::domain::SnapshotInfo,
    },

    /// Confirmation that a snapshot was restored.
    SnapshotRestored {
        /// Container ID that was restored.
        id: String,
        /// Snapshot name that was restored.
        name: String,
    },

    /// List of snapshots for a container.
    SnapshotList {
        /// Container ID.
        id: String,
        /// Available snapshots.
        snapshots: Vec<crate::domain::SnapshotInfo>,
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
    /// Optional human-readable name assigned at creation time.
    #[serde(default)]
    pub name: Option<String>,
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Local alias for the `test_run!` macro that resolves `DaemonRequest` via
    /// `crate::protocol` rather than the cross-crate `minibox_core::protocol` path.
    /// Uses `let`-binding shadowing so field overrides don't produce duplicate-field errors.
    macro_rules! test_run {
        ($($field:ident : $val:expr),* $(,)?) => {{
            #[allow(unused_variables)] let image = "alpine".to_string();
            #[allow(unused_variables)] let tag = None;
            #[allow(unused_variables)] let command = vec!["/bin/sh".to_string()];
            #[allow(unused_variables)] let memory_limit_bytes = None;
            #[allow(unused_variables)] let cpu_weight = None;
            #[allow(unused_variables)] let ephemeral = false;
            #[allow(unused_variables)] let network = None;
            #[allow(unused_variables)] let mounts = vec![];
            #[allow(unused_variables)] let privileged = false;
            #[allow(unused_variables)] let env = vec![];
            #[allow(unused_variables)] let name = None;
            #[allow(unused_variables)] let tty = false;
            #[allow(unused_variables)] let entrypoint = None;
            #[allow(unused_variables)] let user = None;
            #[allow(unused_variables)] let auto_remove = false;
            #[allow(unused_variables)] let priority = None;
            #[allow(unused_variables)] let urgency = None;
            #[allow(unused_variables)] let execution_context = None;
            $(#[allow(unused_variables)] let $field = $val;)*
            crate::protocol::DaemonRequest::Run {
                image, tag, command, memory_limit_bytes, cpu_weight,
                ephemeral, network, mounts, privileged, env, name, tty,
                entrypoint, user, auto_remove, priority, urgency, execution_context,
            }
        }};
    }

    // -----------------------------------------------------------------------
    // Request serialization/deserialization tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_encode_decode_run_request_minimal() {
        let req = test_run!();

        let encoded = encode_request(&req).expect("encode failed");
        let decoded = decode_request(&encoded).expect("decode failed");

        match decoded {
            DaemonRequest::Run {
                image,
                tag,
                command,
                memory_limit_bytes,
                cpu_weight,
                ..
            } => {
                assert_eq!(image, "alpine");
                assert_eq!(tag, None);
                assert_eq!(command, vec!["/bin/sh"]);
                assert_eq!(memory_limit_bytes, None);
                assert_eq!(cpu_weight, None);
            }
            _ => panic!("wrong request type"),
        }
    }

    #[test]
    fn test_encode_decode_run_request_with_limits() {
        let req = test_run!(
            image: "ubuntu".to_string(),
            tag: Some("22.04".to_string()),
            command: vec!["/bin/bash".to_string(), "-c".to_string(), "echo hi".to_string()],
            memory_limit_bytes: Some(536870912),
            cpu_weight: Some(500),
        );

        let encoded = encode_request(&req).expect("encode failed");
        let decoded = decode_request(&encoded).expect("decode failed");

        match decoded {
            DaemonRequest::Run {
                image,
                tag,
                command,
                memory_limit_bytes,
                cpu_weight,
                ..
            } => {
                assert_eq!(image, "ubuntu");
                assert_eq!(tag, Some("22.04".to_string()));
                assert_eq!(command, vec!["/bin/bash", "-c", "echo hi"]);
                assert_eq!(memory_limit_bytes, Some(536870912));
                assert_eq!(cpu_weight, Some(500));
            }
            _ => panic!("wrong request type"),
        }
    }

    #[test]
    fn test_encode_decode_stop_request() {
        let req = DaemonRequest::Stop {
            id: "abc123".to_string(),
        };

        let encoded = encode_request(&req).expect("encode failed");
        let decoded = decode_request(&encoded).expect("decode failed");

        match decoded {
            DaemonRequest::Stop { id } => {
                assert_eq!(id, "abc123");
            }
            _ => panic!("wrong request type"),
        }
    }

    #[test]
    fn test_encode_decode_remove_request() {
        let req = DaemonRequest::Remove {
            id: "xyz789".to_string(),
        };

        let encoded = encode_request(&req).expect("encode failed");
        let decoded = decode_request(&encoded).expect("decode failed");

        match decoded {
            DaemonRequest::Remove { id } => {
                assert_eq!(id, "xyz789");
            }
            _ => panic!("wrong request type"),
        }
    }

    #[test]
    fn test_encode_decode_list_request() {
        let req = DaemonRequest::List;

        let encoded = encode_request(&req).expect("encode failed");
        let decoded = decode_request(&encoded).expect("decode failed");

        assert!(matches!(decoded, DaemonRequest::List));
    }

    #[test]
    fn test_encode_decode_pull_request() {
        let req = DaemonRequest::Pull {
            image: "nginx".to_string(),
            tag: Some("alpine".to_string()),
        };

        let encoded = encode_request(&req).expect("encode failed");
        let decoded = decode_request(&encoded).expect("decode failed");

        match decoded {
            DaemonRequest::Pull { image, tag } => {
                assert_eq!(image, "nginx");
                assert_eq!(tag, Some("alpine".to_string()));
            }
            _ => panic!("wrong request type"),
        }
    }

    // -----------------------------------------------------------------------
    // Response serialization/deserialization tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_encode_decode_container_created_response() {
        let resp = DaemonResponse::ContainerCreated {
            id: "container123".to_string(),
        };

        let encoded = encode_response(&resp).expect("encode failed");
        let decoded = decode_response(&encoded).expect("decode failed");

        match decoded {
            DaemonResponse::ContainerCreated { id } => {
                assert_eq!(id, "container123");
            }
            _ => panic!("wrong response type"),
        }
    }

    #[test]
    fn test_encode_decode_success_response() {
        let resp = DaemonResponse::Success {
            message: "container stopped".to_string(),
        };

        let encoded = encode_response(&resp).expect("encode failed");
        let decoded = decode_response(&encoded).expect("decode failed");

        match decoded {
            DaemonResponse::Success { message } => {
                assert_eq!(message, "container stopped");
            }
            _ => panic!("wrong response type"),
        }
    }

    #[test]
    fn test_encode_decode_error_response() {
        let resp = DaemonResponse::Error {
            message: "container not found".to_string(),
        };

        let encoded = encode_response(&resp).expect("encode failed");
        let decoded = decode_response(&encoded).expect("decode failed");

        match decoded {
            DaemonResponse::Error { message } => {
                assert_eq!(message, "container not found");
            }
            _ => panic!("wrong response type"),
        }
    }

    #[test]
    fn test_encode_decode_container_list_response() {
        let containers = vec![
            ContainerInfo {
                id: "abc123".to_string(),
                name: None,
                image: "alpine:latest".to_string(),
                command: "/bin/sh".to_string(),
                state: "running".to_string(),
                created_at: "2026-03-15T12:00:00Z".to_string(),
                pid: Some(1234),
            },
            ContainerInfo {
                id: "def456".to_string(),
                name: None,
                image: "ubuntu:22.04".to_string(),
                command: "/bin/bash".to_string(),
                state: "stopped".to_string(),
                created_at: "2026-03-15T11:00:00Z".to_string(),
                pid: None,
            },
        ];

        let resp = DaemonResponse::ContainerList {
            containers: containers.clone(),
        };

        let encoded = encode_response(&resp).expect("encode failed");
        let decoded = decode_response(&encoded).expect("decode failed");

        match decoded {
            DaemonResponse::ContainerList {
                containers: decoded_containers,
            } => {
                assert_eq!(decoded_containers.len(), 2);
                assert_eq!(decoded_containers[0].id, "abc123");
                assert_eq!(decoded_containers[0].pid, Some(1234));
                assert_eq!(decoded_containers[1].id, "def456");
                assert_eq!(decoded_containers[1].pid, None);
            }
            _ => panic!("wrong response type"),
        }
    }

    #[test]
    fn test_request_json_has_type_tag() {
        let req = test_run!();

        let encoded = encode_request(&req).expect("encode failed");
        let json_str = String::from_utf8_lossy(&encoded);

        assert!(json_str.contains("\"type\":\"Run\""));
    }

    #[test]
    fn test_response_json_has_type_tag() {
        let resp = DaemonResponse::Success {
            message: "ok".to_string(),
        };

        let encoded = encode_response(&resp).expect("encode failed");
        let json_str = String::from_utf8_lossy(&encoded);

        assert!(json_str.contains("\"type\":\"Success\""));
    }

    #[test]
    fn test_decode_request_strips_newline() {
        let json_with_newline = b"{\"type\":\"List\"}\n";
        let json_without_newline = b"{\"type\":\"List\"}";

        let decoded_with = decode_request(json_with_newline).expect("decode with newline failed");
        let decoded_without =
            decode_request(json_without_newline).expect("decode without newline failed");

        assert!(matches!(decoded_with, DaemonRequest::List));
        assert!(matches!(decoded_without, DaemonRequest::List));
    }

    #[test]
    fn test_decode_response_strips_newline() {
        let json_with_newline = b"{\"type\":\"Success\",\"message\":\"ok\"}\n";
        let json_without_newline = b"{\"type\":\"Success\",\"message\":\"ok\"}";

        let decoded_with = decode_response(json_with_newline).expect("decode with newline failed");
        let decoded_without =
            decode_response(json_without_newline).expect("decode without newline failed");

        assert!(matches!(decoded_with, DaemonResponse::Success { .. }));
        assert!(matches!(decoded_without, DaemonResponse::Success { .. }));
    }

    #[test]
    fn test_decode_malformed_json_fails() {
        let malformed = b"{\"type\":\"Run\",\"image\":";
        let result = decode_request(malformed);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_unknown_type_fails() {
        let unknown_type = b"{\"type\":\"UnknownCommand\"}";
        let result = decode_request(unknown_type);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_missing_required_field_fails() {
        // Run request missing required "image" field
        let missing_field = b"{\"type\":\"Run\",\"command\":[\"/bin/sh\"]}";
        let result = decode_request(missing_field);
        assert!(result.is_err());
    }

    #[test]
    fn test_run_request_empty_command() {
        let req = test_run!(command: Vec::<String>::new());

        let encoded = encode_request(&req).expect("encode failed");
        let decoded = decode_request(&encoded).expect("decode failed");

        match decoded {
            DaemonRequest::Run { command, .. } => {
                assert_eq!(command.len(), 0);
            }
            _ => panic!("wrong request type"),
        }
    }

    #[test]
    fn test_run_request_max_memory_limit() {
        let req = test_run!(memory_limit_bytes: Some(u64::MAX));

        let encoded = encode_request(&req).expect("encode failed");
        let decoded = decode_request(&encoded).expect("decode failed");

        match decoded {
            DaemonRequest::Run {
                memory_limit_bytes, ..
            } => {
                assert_eq!(memory_limit_bytes, Some(u64::MAX));
            }
            _ => panic!("wrong request type"),
        }
    }

    #[test]
    fn test_container_info_special_characters() {
        let info = ContainerInfo {
            id: "abc123".to_string(),
            name: None,
            image: "test/image:v1.0-alpha".to_string(),
            command: "/bin/sh -c 'echo \"hello world\"'".to_string(),
            state: "running".to_string(),
            created_at: "2026-03-15T12:00:00Z".to_string(),
            pid: Some(1234),
        };

        let resp = DaemonResponse::ContainerList {
            containers: vec![info],
        };

        let encoded = encode_response(&resp).expect("encode failed");
        let decoded = decode_response(&encoded).expect("decode failed");

        match decoded {
            DaemonResponse::ContainerList { containers } => {
                assert_eq!(containers[0].command, "/bin/sh -c 'echo \"hello world\"'");
            }
            _ => panic!("wrong response type"),
        }
    }

    #[test]
    fn test_encoded_message_ends_with_newline() {
        let req = DaemonRequest::List;
        let encoded = encode_request(&req).expect("encode failed");

        assert_eq!(encoded.last(), Some(&b'\n'));
    }

    // -----------------------------------------------------------------------
    // Streaming / ephemeral protocol tests (Task 5)
    // -----------------------------------------------------------------------

    #[test]
    fn run_request_defaults_ephemeral_false() {
        let json = r#"{"type":"Run","image":"alpine","tag":"latest","command":["sh"],"memory_limit_bytes":null,"cpu_weight":null}"#;
        let req: DaemonRequest = serde_json::from_str(json).unwrap();
        match req {
            DaemonRequest::Run { ephemeral, .. } => assert!(!ephemeral),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn run_request_explicit_ephemeral_true() {
        let json = r#"{"type":"Run","image":"alpine","tag":"latest","command":["sh"],"memory_limit_bytes":null,"cpu_weight":null,"ephemeral":true}"#;
        let req: DaemonRequest = serde_json::from_str(json).unwrap();
        match req {
            DaemonRequest::Run { ephemeral, .. } => assert!(ephemeral),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn container_output_roundtrip() {
        let msg = DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data: "aGVsbG8=".to_owned(), // base64("hello")
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: DaemonResponse = serde_json::from_str(&json).unwrap();
        match back {
            DaemonResponse::ContainerOutput { stream, data } => {
                assert_eq!(stream, OutputStreamKind::Stdout);
                assert_eq!(data, "aGVsbG8=");
            }
            _ => panic!("expected ContainerOutput"),
        }
    }

    #[test]
    fn container_stopped_roundtrip() {
        let msg = DaemonResponse::ContainerStopped { exit_code: 42 };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"exit_code\":42"));
        let back: DaemonResponse = serde_json::from_str(&json).unwrap();
        match back {
            DaemonResponse::ContainerStopped { exit_code } => assert_eq!(exit_code, 42),
            _ => panic!("expected ContainerStopped"),
        }
    }

    #[test]
    fn output_stream_kind_serde_lowercase() {
        let stdout = serde_json::to_string(&OutputStreamKind::Stdout).unwrap();
        let stderr = serde_json::to_string(&OutputStreamKind::Stderr).unwrap();
        assert_eq!(stdout, r#""stdout""#);
        assert_eq!(stderr, r#""stderr""#);
    }

    #[test]
    fn run_request_with_network_mode_roundtrip() {
        use crate::domain::NetworkMode;
        let req = test_run!(network: Some(NetworkMode::Host));
        let encoded = encode_request(&req).expect("encode");
        let decoded = decode_request(&encoded).expect("decode");
        match decoded {
            DaemonRequest::Run { network, .. } => assert_eq!(network, Some(NetworkMode::Host)),
            _ => panic!("wrong request type"),
        }
    }

    #[test]
    fn run_request_without_network_defaults_to_none_option() {
        let json = r#"{"type":"Run","image":"alpine","command":["sh"],"memory_limit_bytes":null,"cpu_weight":null}"#;
        let req: DaemonRequest = serde_json::from_str(json).expect("parse");
        match req {
            DaemonRequest::Run { network, .. } => assert_eq!(network, None),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn run_request_with_mounts_roundtrip() {
        use crate::domain::BindMount;
        use std::path::PathBuf;
        let req = test_run!(
            image: "ubuntu".to_string(),
            mounts: vec![BindMount {
                host_path: PathBuf::from("/tmp/foo"),
                container_path: PathBuf::from("/bar"),
                read_only: false,
            }],
        );
        let encoded = encode_request(&req).unwrap();
        let decoded = decode_request(&encoded).unwrap();
        match decoded {
            DaemonRequest::Run {
                mounts, privileged, ..
            } => {
                assert_eq!(mounts.len(), 1);
                assert_eq!(mounts[0].host_path, PathBuf::from("/tmp/foo"));
                assert_eq!(mounts[0].container_path, PathBuf::from("/bar"));
                assert!(!mounts[0].read_only);
                assert!(!privileged);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn run_request_privileged_roundtrip() {
        let req = DaemonRequest::Run {
            image: "ubuntu".to_string(),
            tag: None,
            command: vec!["/bin/sh".to_string()],
            memory_limit_bytes: None,
            cpu_weight: None,
            ephemeral: false,
            network: None,
            mounts: vec![],
            privileged: true,
            env: vec![],
            name: None,
            tty: false,
            entrypoint: None,
            user: None,
            auto_remove: false,
            priority: None,
            urgency: None,
            execution_context: None,
        };
        let encoded = encode_request(&req).unwrap();
        let decoded = decode_request(&encoded).unwrap();
        match decoded {
            DaemonRequest::Run {
                privileged, mounts, ..
            } => {
                assert!(privileged);
                assert!(mounts.is_empty());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn container_logs_request_roundtrip() {
        let req = DaemonRequest::ContainerLogs {
            container_id: "abc123".to_string(),
            follow: false,
        };
        let encoded = encode_request(&req).expect("encode");
        let decoded = decode_request(&encoded).expect("decode");
        match decoded {
            DaemonRequest::ContainerLogs {
                container_id,
                follow,
            } => {
                assert_eq!(container_id, "abc123");
                assert!(!follow);
            }
            _ => panic!("expected ContainerLogs"),
        }
    }

    #[test]
    fn container_logs_request_follow_defaults_false() {
        let json = r#"{"type":"ContainerLogs","container_id":"abc"}"#;
        let req: DaemonRequest = serde_json::from_str(json).expect("parse");
        match req {
            DaemonRequest::ContainerLogs { follow, .. } => assert!(!follow),
            _ => panic!("expected ContainerLogs"),
        }
    }

    #[test]
    fn log_line_response_roundtrip() {
        let resp = DaemonResponse::LogLine {
            stream: OutputStreamKind::Stderr,
            line: "error: something bad".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialize");
        assert!(json.contains("\"type\":\"LogLine\""));
        let back: DaemonResponse = serde_json::from_str(&json).expect("deserialize");
        match back {
            DaemonResponse::LogLine { stream, line } => {
                assert_eq!(stream, OutputStreamKind::Stderr);
                assert_eq!(line, "error: something bad");
            }
            _ => panic!("expected LogLine"),
        }
    }

    #[test]
    fn run_request_old_json_without_mounts_defaults() {
        // Old clients that don't send mounts/privileged must still deserialize.
        let json = r#"{"type":"Run","image":"alpine","command":["sh"],"memory_limit_bytes":null,"cpu_weight":null}"#;
        let req: DaemonRequest = serde_json::from_str(json).unwrap();
        match req {
            DaemonRequest::Run {
                mounts, privileged, ..
            } => {
                assert!(mounts.is_empty());
                assert!(!privileged);
            }
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn send_input_roundtrip() {
        use base64::Engine as _;
        let bytes = b"ls\n";
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        let req = DaemonRequest::SendInput {
            session_id: crate::domain::SessionId::from("sess1"),
            data: data.clone(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"SendInput\""), "{json}");
        let back: DaemonRequest = serde_json::from_str(&json).unwrap();
        match back {
            DaemonRequest::SendInput { data: d, .. } => assert_eq!(d, data),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn resize_pty_roundtrip() {
        let req = DaemonRequest::ResizePty {
            session_id: crate::domain::SessionId::from("sess1"),
            cols: 120,
            rows: 40,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"ResizePty\""), "{json}");
        let back: DaemonRequest = serde_json::from_str(&json).unwrap();
        match back {
            DaemonRequest::ResizePty { cols, rows, .. } => {
                assert_eq!(cols, 120);
                assert_eq!(rows, 40);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn run_request_tty_defaults_false() {
        // Old clients omitting `tty` must still deserialise cleanly.
        let json = r#"{"type":"Run","image":"alpine","tag":"latest","command":["sh"],"memory_limit_bytes":null,"cpu_weight":null}"#;
        let req: DaemonRequest = serde_json::from_str(json).unwrap();
        match req {
            DaemonRequest::Run { tty, .. } => assert!(!tty),
            _ => panic!("wrong variant"),
        }
    }

    // -----------------------------------------------------------------------
    // Wire format snapshot tests — catch serialization drift
    //
    // Each test pins the exact JSON representation of a protocol variant.
    // If the serde output changes (field renamed, tag changed, etc.) these
    // tests fail immediately rather than silently breaking the wire protocol.
    // -----------------------------------------------------------------------

    #[test]
    fn wire_snapshot_run_request() {
        let req = DaemonRequest::Run {
            image: "library/alpine".to_string(),
            tag: Some("3.18".to_string()),
            command: vec!["sh".to_string(), "-c".to_string(), "echo hi".to_string()],
            memory_limit_bytes: Some(134217728),
            cpu_weight: Some(100),
            ephemeral: true,
            network: None,
            env: vec!["FOO=bar".to_string()],
            mounts: vec![],
            privileged: false,
            name: Some("my-container".to_string()),
            tty: false,
            entrypoint: None,
            user: None,
            auto_remove: false,
            priority: None,
            urgency: None,
            execution_context: None,
        };
        let json = serde_json::to_string(&req).expect("serialize");
        // Pin the type discriminant and required fields.
        assert!(json.contains("\"type\":\"Run\""), "type tag: {json}");
        assert!(
            json.contains("\"image\":\"library/alpine\""),
            "image: {json}"
        );
        assert!(json.contains("\"tag\":\"3.18\""), "tag: {json}");
        assert!(json.contains("\"ephemeral\":true"), "ephemeral: {json}");
        assert!(
            json.contains("\"memory_limit_bytes\":134217728"),
            "memory: {json}"
        );
        assert!(json.contains("\"cpu_weight\":100"), "cpu: {json}");
        assert!(json.contains("\"name\":\"my-container\""), "name: {json}");
        // Verify it round-trips back to the same JSON.
        let decoded: DaemonRequest = serde_json::from_str(&json).expect("deserialize");
        let re_encoded = serde_json::to_string(&decoded).expect("re-serialize");
        assert_eq!(
            json, re_encoded,
            "wire format is not stable across roundtrip"
        );
    }

    #[test]
    fn wire_snapshot_stop_request() {
        let json = serde_json::to_string(&DaemonRequest::Stop {
            id: "abc123def456".to_string(),
        })
        .expect("serialize");
        assert_eq!(json, r#"{"type":"Stop","id":"abc123def456"}"#);
    }

    #[test]
    fn wire_snapshot_list_request() {
        let json = serde_json::to_string(&DaemonRequest::List).expect("serialize");
        assert_eq!(json, r#"{"type":"List"}"#);
    }

    #[test]
    fn wire_snapshot_pull_request() {
        let json = serde_json::to_string(&DaemonRequest::Pull {
            image: "library/nginx".to_string(),
            tag: Some("stable".to_string()),
        })
        .expect("serialize");
        assert_eq!(
            json,
            r#"{"type":"Pull","image":"library/nginx","tag":"stable"}"#
        );
    }

    #[test]
    fn wire_snapshot_container_created_response() {
        let json = serde_json::to_string(&DaemonResponse::ContainerCreated {
            id: "deadbeef1234".to_string(),
        })
        .expect("serialize");
        assert_eq!(json, r#"{"type":"ContainerCreated","id":"deadbeef1234"}"#);
    }

    #[test]
    fn wire_snapshot_success_response() {
        let json = serde_json::to_string(&DaemonResponse::Success {
            message: "stopped".to_string(),
        })
        .expect("serialize");
        assert_eq!(json, r#"{"type":"Success","message":"stopped"}"#);
    }

    #[test]
    fn wire_snapshot_error_response() {
        let json = serde_json::to_string(&DaemonResponse::Error {
            message: "container not found".to_string(),
        })
        .expect("serialize");
        assert_eq!(json, r#"{"type":"Error","message":"container not found"}"#);
    }

    #[test]
    fn wire_snapshot_container_output_stdout() {
        let json = serde_json::to_string(&DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data: "aGVsbG8=".to_string(),
        })
        .expect("serialize");
        assert_eq!(
            json,
            r#"{"type":"ContainerOutput","stream":"stdout","data":"aGVsbG8="}"#
        );
    }

    #[test]
    fn wire_snapshot_container_output_stderr() {
        let json = serde_json::to_string(&DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stderr,
            data: "ZXJyb3I=".to_string(),
        })
        .expect("serialize");
        assert_eq!(
            json,
            r#"{"type":"ContainerOutput","stream":"stderr","data":"ZXJyb3I="}"#
        );
    }

    #[test]
    fn wire_snapshot_container_stopped_response() {
        let json = serde_json::to_string(&DaemonResponse::ContainerStopped { exit_code: 0 })
            .expect("serialize");
        assert_eq!(json, r#"{"type":"ContainerStopped","exit_code":0}"#);
    }

    #[test]
    fn wire_snapshot_container_stopped_nonzero_exit() {
        let json = serde_json::to_string(&DaemonResponse::ContainerStopped { exit_code: 137 })
            .expect("serialize");
        assert_eq!(json, r#"{"type":"ContainerStopped","exit_code":137}"#);
    }

    #[test]
    fn wire_snapshot_push_credentials_anonymous() {
        let json = serde_json::to_string(&PushCredentials::Anonymous).expect("serialize");
        assert_eq!(json, r#"{"type":"Anonymous"}"#);
    }

    #[test]
    fn wire_snapshot_push_credentials_basic() {
        let json = serde_json::to_string(&PushCredentials::Basic {
            username: "user".to_string(),
            password: "s3cr3t".to_string(),
        })
        .expect("serialize");
        assert_eq!(
            json,
            r#"{"type":"Basic","username":"user","password":"s3cr3t"}"#
        );
    }

    #[test]
    fn wire_snapshot_send_input_request() {
        use crate::domain::SessionId;
        let json = serde_json::to_string(&DaemonRequest::SendInput {
            session_id: SessionId::from("sess-abc"),
            data: "bHMK".to_string(),
        })
        .expect("serialize");
        assert!(json.contains("\"type\":\"SendInput\""), "{json}");
        assert!(json.contains("\"session_id\":\"sess-abc\""), "{json}");
        assert!(json.contains("\"data\":\"bHMK\""), "{json}");
    }

    #[test]
    fn wire_snapshot_resize_pty_request() {
        use crate::domain::SessionId;
        let json = serde_json::to_string(&DaemonRequest::ResizePty {
            session_id: SessionId::from("sess-abc"),
            cols: 200,
            rows: 50,
        })
        .expect("serialize");
        assert!(json.contains("\"type\":\"ResizePty\""), "{json}");
        assert!(json.contains("\"cols\":200"), "{json}");
        assert!(json.contains("\"rows\":50"), "{json}");
    }

    /// Verify that all protocol field names have not changed since last known
    /// good state. Any addition of a field with `#[serde(default)]` is fine
    /// (old clients omitting it still decode). Renaming a field is a breaking
    /// change and will cause this test to fail.
    #[test]
    fn wire_format_run_request_field_names_stable() {
        let req = test_run!(image: "i".to_string(), command: Vec::<String>::new());
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&req).expect("serialize")).expect("parse");
        let obj = v.as_object().expect("object");
        // Check every field name that must exist in the wire format.
        for field in &[
            "type",
            "image",
            "tag",
            "command",
            "memory_limit_bytes",
            "cpu_weight",
            "ephemeral",
            "network",
            "env",
            "mounts",
            "privileged",
            "name",
            "tty",
        ] {
            assert!(
                obj.contains_key(*field),
                "missing field {field} in Run wire format"
            );
        }
    }
}
