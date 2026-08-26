//! Domain ports and value types for executing commands in containers.

use async_trait::async_trait;
use std::sync::Arc;

use super::{AsAny, ContainerId};

// ---------------------------------------------------------------------------
// Exec Runtime Port
// ---------------------------------------------------------------------------

/// Pure specification for running a command inside a container.
///
/// This is a domain value type — no channel fields, no tokio types.
/// Channel wiring (stdin relay, PTY resize) belongs in the infrastructure
/// adapter layer (`minibox::adapters::exec`).
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

// ---------------------------------------------------------------------------
// ProgressSink — runtime-agnostic channel abstraction (#278)
// ---------------------------------------------------------------------------

/// An async-capable sink for streaming progress updates from domain ports.
///
/// Replaces direct `tokio::sync::mpsc::Sender<T>` parameters in port trait
/// signatures so the domain layer is not coupled to a specific async runtime.
/// Adapters (and tests) provide concrete implementations — the blanket impl
/// for `tokio::sync::mpsc::Sender<T>` covers the production case.
#[async_trait]
pub trait ProgressSink<T: Send + 'static>: Send + Sync {
    /// Send a value into the sink.
    ///
    /// Returns `Ok(())` when the value was accepted, or `Err(())` when the
    /// receiver has been dropped (analogous to `mpsc::SendError`).
    async fn send(&self, value: T) -> Result<(), ()>;
}

/// Blanket implementation so `tokio::sync::mpsc::Sender<T>` satisfies
/// `ProgressSink<T>` without wrapper code at every call site.
#[async_trait]
impl<T: Send + 'static> ProgressSink<T> for tokio::sync::mpsc::Sender<T> {
    async fn send(&self, value: T) -> Result<(), ()> {
        Self::send(self, value).await.map_err(|_| ())
    }
}

/// Blanket implementation so `Arc<dyn ProgressSink<T>>` (i.e. `DynProgressSink<T>`)
/// can be passed where `&dyn ProgressSink<T>` is expected.
#[async_trait]
impl<T: Send + 'static> ProgressSink<T> for Arc<dyn ProgressSink<T>> {
    async fn send(&self, value: T) -> Result<(), ()> {
        (**self).send(value).await
    }
}

/// Type-erased progress sink, used in port trait signatures.
///
/// Uses `Arc` rather than `Box` so the sink can be shared across tasks
/// (e.g. a blocking spawn and a forwarding task in the exec adapter).
pub type DynProgressSink<T> = Arc<dyn ProgressSink<T>>;

/// Port for running commands inside already-running containers.
#[async_trait]
pub trait ExecRuntime: AsAny + Send + Sync {
    /// Starts a command in a running container and streams its responses.
    async fn run_in_container(
        &self,
        container_id: &ContainerId,
        spec: ExecSpec,
        tx: DynProgressSink<crate::protocol::DaemonResponse>,
    ) -> anyhow::Result<ExecHandle>;
}

/// Type alias for a shared, dynamic [`ExecRuntime`] implementation.
pub type DynExecRuntime = Arc<dyn ExecRuntime>;
