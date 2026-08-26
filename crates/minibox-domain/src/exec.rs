//! Domain ports and value types for executing commands in containers.

use async_trait::async_trait;
use std::sync::Arc;

use crate::{AsAny, ContainerId};

/// Pure specification for running a command inside a container.
#[derive(Debug, Clone)]
pub struct ExecSpec {
    /// Command and arguments to execute.
    pub cmd: Vec<String>,
    /// Environment variables in `KEY=VALUE` form.
    pub env: Vec<String>,
    /// Optional working directory inside the container.
    pub working_dir: Option<std::path::PathBuf>,
    /// Whether to allocate a pseudo-terminal.
    pub tty: bool,
}

/// Handle representing a started exec instance.
#[derive(Debug, Clone)]
pub struct ExecHandle {
    /// Adapter-assigned identifier for the exec instance.
    pub id: String,
}

/// Output produced by an exec adapter before transport encoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecOutput {
    /// Bytes read from standard output.
    Stdout(Vec<u8>),
    /// Bytes read from standard error.
    Stderr(Vec<u8>),
    /// Terminal process exit status.
    Exit(i32),
    /// Adapter failure that occurred after the exec was accepted.
    Error(String),
}

/// Async stream of output from a started exec instance.
#[async_trait]
pub trait ExecOutputStream: Send {
    /// Return the next output value, or `None` when execution has finished.
    async fn next(&mut self) -> Option<ExecOutput>;
}

/// Started exec handle and its bounded output stream.
pub struct ExecSession {
    /// Adapter-assigned handle delivered before any output is forwarded.
    pub handle: ExecHandle,
    /// Output stream consumed by the transport boundary after announcing the handle.
    pub output: Box<dyn ExecOutputStream>,
}

impl std::fmt::Debug for ExecSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecSession")
            .field("handle", &self.handle)
            .finish_non_exhaustive()
    }
}

/// Port for running commands inside already-running containers.
#[async_trait]
pub trait ExecRuntime: AsAny + Send + Sync {
    /// Start a command and stream domain output values.
    ///
    /// Implementations return a session before callers consume output, making
    /// handle delivery an explicit ordering boundary while preserving bounded
    /// backpressure in the adapter-owned stream.
    async fn run_in_container(
        &self,
        container_id: &ContainerId,
        spec: ExecSpec,
    ) -> anyhow::Result<ExecSession>;
}

/// Shared dynamic exec runtime.
pub type DynExecRuntime = Arc<dyn ExecRuntime>;
