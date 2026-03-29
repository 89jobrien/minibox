//! Extended container functionality domain traits.
//!
//! Defines contracts for TTY support, exec, logs, and persistent state.

use super::AsAny;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// TTY Support
// ---------------------------------------------------------------------------

/// Abstraction for TTY (terminal) support in containers.
///
/// Enables interactive shells with proper terminal emulation.
///
/// # Example
///
/// ```rust,ignore
/// let tty = PseudoTerminal::new();
/// let config = TtyConfig {
///     width: 80,
///     height: 24,
///     term: "xterm-256color".to_string(),
/// };
///
/// let (master_fd, slave_fd) = tty.create(&config)?;
/// // Attach slave_fd to container stdin/stdout/stderr
/// // Forward I/O between master_fd and client
/// ```
#[async_trait]
pub trait TtyProvider: AsAny + Send + Sync {
    /// Create a pseudo-terminal for a container.
    ///
    /// Returns (master_fd, slave_fd) file descriptors.
    ///
    /// # Arguments
    ///
    /// * `config` - Terminal configuration
    ///
    /// # Returns
    ///
    /// Tuple of (master file descriptor, slave file descriptor).
    /// Master is used by daemon for I/O forwarding.
    /// Slave is attached to container process.
    async fn create(&self, config: &TtyConfig) -> Result<(i32, i32)>;

    /// Resize an existing terminal.
    ///
    /// # Arguments
    ///
    /// * `master_fd` - Master file descriptor
    /// * `width` - New width in columns
    /// * `height` - New height in rows
    async fn resize(&self, master_fd: i32, width: u16, height: u16) -> Result<()>;

    /// Close a terminal and release resources.
    async fn close(&self, master_fd: i32) -> Result<()>;
}

/// Terminal configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtyConfig {
    /// Terminal width in columns
    pub width: u16,

    /// Terminal height in rows
    pub height: u16,

    /// TERM environment variable value
    pub term: String,
}

impl Default for TtyConfig {
    fn default() -> Self {
        Self {
            width: 80,
            height: 24,
            term: "xterm-256color".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Exec Support
// ---------------------------------------------------------------------------

/// Abstraction for executing commands in running containers.
///
/// Enables `minibox exec <container> <command>` functionality.
///
/// # Example
///
/// ```rust,ignore
/// let exec = ContainerExec::new();
/// let config = ExecConfig {
///     command: "/bin/bash".to_string(),
///     args: vec!["-c".to_string(), "ls -la".to_string()],
///     env: vec![],
///     working_dir: Some("/app".to_string()),
///     tty: false,
///     user: None,
/// };
///
/// let exec_id = exec.create("container-abc123", &config).await?;
/// let output = exec.start(exec_id).await?;
/// ```
#[async_trait]
pub trait ExecProvider: AsAny + Send + Sync {
    /// Create an exec session for a running container.
    ///
    /// Prepares to execute a command but doesn't start it yet.
    ///
    /// # Arguments
    ///
    /// * `container_id` - Target container identifier
    /// * `config` - Exec configuration
    ///
    /// # Returns
    ///
    /// Exec session identifier for later start/attach operations.
    async fn create(&self, container_id: &str, config: &ExecConfig) -> Result<String>;

    /// Start an exec session.
    ///
    /// Enters the container's namespaces and executes the command.
    ///
    /// # Arguments
    ///
    /// * `exec_id` - Exec session identifier from create()
    ///
    /// # Returns
    ///
    /// Process ID of the exec'd command.
    async fn start(&self, exec_id: &str) -> Result<u32>;

    /// Inspect an exec session.
    ///
    /// Returns current state and metadata.
    async fn inspect(&self, exec_id: &str) -> Result<ExecInfo>;

    /// Cleanup an exec session.
    async fn cleanup(&self, exec_id: &str) -> Result<()>;
}

/// Configuration for exec session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecConfig {
    /// Command to execute
    pub command: String,

    /// Command arguments
    pub args: Vec<String>,

    /// Additional environment variables
    pub env: Vec<String>,

    /// Working directory (None = container's default)
    pub working_dir: Option<String>,

    /// Allocate TTY
    pub tty: bool,

    /// User to run as (None = container's default user)
    pub user: Option<String>,

    /// Detach after starting (background execution)
    pub detach: bool,
}

/// Information about an exec session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecInfo {
    /// Exec session ID
    pub id: String,

    /// Container ID
    pub container_id: String,

    /// Executed command
    pub command: String,

    /// Whether session is running
    pub running: bool,

    /// Exit code (if completed)
    pub exit_code: Option<i32>,

    /// Process ID (if running)
    pub pid: Option<u32>,
}

// ---------------------------------------------------------------------------
// Logs Support
// ---------------------------------------------------------------------------

/// Abstraction for container log capture and streaming.
///
/// Enables `minibox logs <container>` functionality.
#[async_trait]
pub trait LogProvider: AsAny + Send + Sync {
    /// Get logs for a container.
    ///
    /// # Arguments
    ///
    /// * `container_id` - Container identifier
    /// * `config` - Log retrieval options
    ///
    /// # Returns
    ///
    /// Log lines from container stdout/stderr.
    async fn get(&self, container_id: &str, config: &LogConfig) -> Result<Vec<LogLine>>;

    /// Stream logs from a container.
    ///
    /// Returns a channel that emits log lines as they're written.
    async fn stream(&self, container_id: &str, config: &LogConfig) -> Result<LogStream>;

    /// Clear logs for a container.
    async fn clear(&self, container_id: &str) -> Result<()>;
}

/// Log retrieval configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// Follow log output (tail -f behavior)
    pub follow: bool,

    /// Show stdout
    pub stdout: bool,

    /// Show stderr
    pub stderr: bool,

    /// Only show logs since timestamp (Unix seconds)
    pub since: Option<i64>,

    /// Only show logs until timestamp (Unix seconds)
    pub until: Option<i64>,

    /// Number of lines from end to show (0 = all)
    pub tail: usize,

    /// Show timestamps
    pub timestamps: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            follow: false,
            stdout: true,
            stderr: true,
            since: None,
            until: None,
            tail: 0,
            timestamps: false,
        }
    }
}

/// A single log line from a container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    /// Timestamp (Unix nanoseconds)
    pub timestamp: i64,

    /// Stream type (stdout/stderr)
    pub stream: LogStream,

    /// Log message content
    pub message: String,
}

/// Log stream type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogStream {
    /// Standard output
    Stdout,
    /// Standard error
    Stderr,
}

// ---------------------------------------------------------------------------
// Persistent State
// ---------------------------------------------------------------------------

/// Abstraction for persistent container state storage.
///
/// Enables daemon restart without losing container information.
///
/// # Example
///
/// ```rust,ignore
/// let state_store = SqliteStateStore::new("/var/lib/minibox/state.db")?;
///
/// // On container create
/// state_store.save("container-abc123", &container_info).await?;
///
/// // On daemon restart
/// let containers = state_store.load_all().await?;
/// for container in containers {
///     // Restore container state
/// }
/// ```
#[async_trait]
pub trait StateStore: AsAny + Send + Sync {
    /// Save container state to persistent storage.
    ///
    /// # Arguments
    ///
    /// * `container_id` - Container identifier
    /// * `info` - Container metadata to persist
    async fn save(&self, container_id: &str, info: &PersistentContainerInfo) -> Result<()>;

    /// Load container state from persistent storage.
    async fn load(&self, container_id: &str) -> Result<Option<PersistentContainerInfo>>;

    /// Load all container states.
    async fn load_all(&self) -> Result<Vec<PersistentContainerInfo>>;

    /// Delete container state.
    async fn delete(&self, container_id: &str) -> Result<()>;

    /// Update container state.
    async fn update_state(&self, container_id: &str, new_state: &str) -> Result<()>;
}

/// Persistent container information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentContainerInfo {
    /// Container ID
    pub id: String,

    /// Image name
    pub image: String,

    /// Image tag
    pub tag: String,

    /// Command
    pub command: String,

    /// Arguments
    pub args: Vec<String>,

    /// State (Created, Running, Stopped, Failed)
    pub state: String,

    /// Process ID (if running)
    pub pid: Option<u32>,

    /// Creation timestamp (Unix seconds)
    pub created_at: i64,

    /// Started timestamp (Unix seconds)
    pub started_at: Option<i64>,

    /// Stopped timestamp (Unix seconds)
    pub stopped_at: Option<i64>,

    /// Exit code (if stopped)
    pub exit_code: Option<i32>,

    /// Container directory path
    pub container_dir: PathBuf,

    /// rootfs path
    pub rootfs: PathBuf,

    /// cgroup path
    pub cgroup_path: PathBuf,

    /// Resource limits applied
    pub resource_config: Option<super::ResourceConfig>,

    /// Network configuration
    pub network_config: Option<super::networking::NetworkConfig>,
}
