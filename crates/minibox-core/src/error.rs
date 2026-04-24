//! Shared and module-specific error types for minibox.
//!
//! Each sub-module has its own fine-grained error type; they all implement
//! `std::error::Error` via thiserror and convert into `anyhow::Error`
//! automatically through the standard `?` operator.
//!
//! This module contains only cross-platform error types. Linux-specific errors
//! that depend on `nix` (FilesystemError, CgroupError, NamespaceError,
//! ProcessError) remain in the `minibox` crate.

use thiserror::Error;

// ---------------------------------------------------------------------------
// Re-exports from minibox-oci
// ---------------------------------------------------------------------------

pub use minibox_oci::error::ImageError;
pub use minibox_oci::error::RegistryError;

// ---------------------------------------------------------------------------
// Top-level error
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum MiniboxError {
    #[error("image not found: {0}")]
    ImageNotFound(String),

    #[error("container not found: {id}")]
    ContainerNotFound { id: String },

    #[error("container is not in the expected state: {0}")]
    InvalidState(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

// ---------------------------------------------------------------------------
// Exec errors
// ---------------------------------------------------------------------------

/// Errors from exec-into-container operations.
#[derive(Debug, Error)]
pub enum ExecError {
    #[error("container {container_id} is not running")]
    ContainerNotRunning { container_id: String },

    #[error("exec {exec_id} not found")]
    ExecNotFound { exec_id: String },

    #[error("nsenter failed for container {container_id}: {reason}")]
    NsenterFailed {
        container_id: String,
        reason: String,
    },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("exec error: {0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// Push errors
// ---------------------------------------------------------------------------

/// Errors from container commit operations.
#[derive(Debug, Error)]
pub enum CommitError {
    #[error("overlay upperdir missing for container {container_id}")]
    UpperdirMissing { container_id: String },

    #[error("layer tar failed: {reason}")]
    LayerTarFailed { reason: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("commit error: {0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// Build errors
// ---------------------------------------------------------------------------

/// Errors from Dockerfile build operations.
#[derive(Debug, Error)]
pub enum BuildError {
    #[error("Dockerfile not found at {path}")]
    DockerfileNotFound { path: String },

    #[error("parse error at line {line}: {reason}")]
    ParseError { line: u32, reason: String },

    #[error("unsupported instruction: {instruction}")]
    UnsupportedInstruction { instruction: String },

    #[error("build step {step} failed with exit code {exit_code}")]
    BuildStepFailed { step: u32, exit_code: i32 },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("build error: {0}")]
    Other(String),
}

/// Errors from OCI image push operations.
#[derive(Debug, Error)]
pub enum PushError {
    #[error("registry authentication failed for {registry}: {message}")]
    AuthFailed { registry: String, message: String },

    #[error("blob upload failed for {digest}: {reason}")]
    BlobUploadFailed { digest: String, reason: String },

    #[error("manifest push failed: {reason}")]
    ManifestPushFailed { reason: String },

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("push error: {0}")]
    Other(String),
}
