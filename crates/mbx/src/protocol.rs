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

use crate::domain::{BindMount, NetworkMode};
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
        /// Environment variables to set inside the container, in `KEY=VALUE` form.
        #[serde(default)]
        env: Vec<String>,
        /// Optional human-readable name for the container.
        ///
        /// When set, the container can be referenced by name in `stop` and `rm`
        /// commands in addition to its auto-generated ID.
        #[serde(default)]
        name: Option<String>,
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
        /// Image name to register (e.g. `"mbx-tester"`).
        name: String,
        /// Image tag to register (e.g. `"latest"`).
        tag: String,
    },

    /// Execute a command inside an already-running container.
    Exec {
        container_id: String,
        cmd: Vec<String>,
        #[serde(default)]
        env: Vec<String>,
        #[serde(default)]
        working_dir: Option<String>,
        #[serde(default)]
        tty: bool,
    },

    /// Push a locally-stored image to a remote OCI registry.
    Push {
        image_ref: String,
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
        /// The image reference that was registered, e.g. `"mbx-tester:latest"`.
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
    ExecStarted {
        exec_id: String,
    },

    /// Push progress update for a single layer.
    ///
    /// Non-terminal: sent zero or more times during a push operation.
    PushProgress {
        layer_digest: String,
        bytes_uploaded: u64,
        total_bytes: u64,
    },

    /// Streaming build log line.
    ///
    /// Non-terminal: sent once per Dockerfile step before `BuildComplete`.
    BuildOutput {
        step: u32,
        total_steps: u32,
        message: String,
    },

    /// Build completed successfully.
    BuildComplete {
        image_id: String,
        tag: String,
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

    // -----------------------------------------------------------------------
    // Request serialization/deserialization tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_encode_decode_run_request_minimal() {
        let req = DaemonRequest::Run {
            image: "alpine".to_string(),
            tag: None,
            command: vec!["/bin/sh".to_string()],
            memory_limit_bytes: None,
            cpu_weight: None,
            ephemeral: false,
            network: None,
            mounts: vec![],
            privileged: false,
            env: vec![],
        };

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
        let req = DaemonRequest::Run {
            image: "ubuntu".to_string(),
            tag: Some("22.04".to_string()),
            command: vec![
                "/bin/bash".to_string(),
                "-c".to_string(),
                "echo hi".to_string(),
            ],
            memory_limit_bytes: Some(536870912), // 512MB
            cpu_weight: Some(500),
            ephemeral: false,
            network: None,
            mounts: vec![],
            privileged: false,
            env: vec![],
        };

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
                image: "alpine:latest".to_string(),
                command: "/bin/sh".to_string(),
                state: "running".to_string(),
                created_at: "2026-03-15T12:00:00Z".to_string(),
                pid: Some(1234),
            },
            ContainerInfo {
                id: "def456".to_string(),
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

    // -----------------------------------------------------------------------
    // JSON format validation tests (security)
    // -----------------------------------------------------------------------

    #[test]
    fn test_request_json_has_type_tag() {
        let req = DaemonRequest::Run {
            image: "alpine".to_string(),
            tag: None,
            command: vec!["/bin/sh".to_string()],
            memory_limit_bytes: None,
            cpu_weight: None,
            ephemeral: false,
            network: None,
            mounts: vec![],
            privileged: false,
            env: vec![],
        };

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

    // -----------------------------------------------------------------------
    // Edge cases and boundary tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_run_request_empty_command() {
        let req = DaemonRequest::Run {
            image: "alpine".to_string(),
            tag: None,
            command: vec![],
            memory_limit_bytes: None,
            cpu_weight: None,
            ephemeral: false,
            network: None,
            mounts: vec![],
            privileged: false,
            env: vec![],
        };

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
        let req = DaemonRequest::Run {
            image: "alpine".to_string(),
            tag: None,
            command: vec!["/bin/sh".to_string()],
            memory_limit_bytes: Some(u64::MAX),
            cpu_weight: None,
            ephemeral: false,
            network: None,
            mounts: vec![],
            privileged: false,
            env: vec![],
        };

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

    // -----------------------------------------------------------------------
    // Network mode protocol tests
    // -----------------------------------------------------------------------

    #[test]
    fn run_request_with_network_mode_roundtrip() {
        use crate::domain::NetworkMode;
        let req = DaemonRequest::Run {
            image: "alpine".to_string(),
            tag: None,
            command: vec!["/bin/sh".to_string()],
            memory_limit_bytes: None,
            cpu_weight: None,
            ephemeral: false,
            network: Some(NetworkMode::Host),
            mounts: vec![],
            privileged: false,
            env: vec![],
        };
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
}
