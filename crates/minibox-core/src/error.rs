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
// Image errors
// ---------------------------------------------------------------------------

/// Errors from image layer extraction and the image store.
#[derive(Debug, Error)]
pub enum ImageError {
    #[error("image {name}:{tag} not found in local store")]
    NotFound { name: String, tag: String },

    #[error("digest mismatch for {digest}: expected {expected}, got {actual}")]
    DigestMismatch {
        digest: String,
        expected: String,
        actual: String,
    },

    #[error("layer extraction failed: {0}")]
    LayerExtract(String),

    /// A tar entry was a block or character device node, which is rejected for
    /// security reasons.  The `entry` field is the path of the offending entry.
    #[error("tar entry is a device node (security rejected): {entry}")]
    DeviceNodeRejected { entry: String },

    /// A tar entry's symlink target contained `..` components after the
    /// absolute->relative rewrite, which would escape the container root.
    #[error(
        "tar entry symlink traverses parent directory (security rejected): {entry} -> {target}"
    )]
    SymlinkTraversalRejected { entry: String, target: String },

    #[error("failed to write to image store at {path}: {source}")]
    StoreWrite {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read from image store at {path}: {source}")]
    StoreRead {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse manifest for {name}:{tag}: {source}")]
    ManifestParse {
        name: String,
        tag: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("layer error: {0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// Registry errors
// ---------------------------------------------------------------------------

/// Errors from the OCI registry client.
#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("authentication failed for {image}: {message}")]
    AuthFailed { image: String, message: String },

    #[error("failed to fetch manifest for {name}:{tag}: {message}")]
    ManifestFetch {
        name: String,
        tag: String,
        message: String,
    },

    #[error("failed to fetch blob {digest}: {message}")]
    BlobFetch { digest: String, message: String },

    #[error("no {platform} manifest found in manifest list")]
    NoPlatformManifest { platform: String },

    #[error("manifest list nesting too deep (max 2 levels)")]
    ManifestNestingTooDeep,

    #[error("layer download task panicked or was cancelled for digest {digest}: {source}")]
    LayerTask {
        digest: String,
        #[source]
        source: tokio::task::JoinError,
    },

    #[error("registry error: {0}")]
    Other(String),
}

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
