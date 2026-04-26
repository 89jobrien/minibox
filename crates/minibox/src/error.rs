//! Shared and module-specific error types for minibox.
//!
//! Each sub-module has its own fine-grained error type; they all implement
//! `std::error::Error` via thiserror and convert into `anyhow::Error`
//! automatically through the standard `?` operator.

use thiserror::Error;

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
// Filesystem errors
// ---------------------------------------------------------------------------

/// Errors from filesystem / overlay / mount operations.
#[derive(Debug, Error)]
pub enum FilesystemError {
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to mount {fs} at {target}: {source}")]
    Mount {
        fs: String,
        target: String,
        #[source]
        source: nix::errno::Errno,
    },

    #[error("overlay mount failed: {0}")]
    OverlayMount(String),

    #[error("pivot_root failed: {0}")]
    PivotRoot(String),

    #[error("failed to unmount {target}: {source}")]
    Umount {
        target: String,
        #[source]
        source: nix::errno::Errno,
    },

    #[error("cleanup failed for {path}: {source}")]
    Cleanup {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

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
    /// absolute→relative rewrite, which would escape the container root.
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

    #[error("no linux/amd64 manifest found in manifest list")]
    NoAmd64Manifest,

    #[error("manifest list nesting too deep (max 2 levels)")]
    ManifestNestingTooDeep,

    #[error("registry error: {0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// Cgroup errors
// ---------------------------------------------------------------------------

/// Errors from cgroup v2 operations.
#[derive(Debug, Error)]
pub enum CgroupError {
    #[error("failed to create cgroup directory {path}: {source}")]
    CreateFailed {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to add process {pid} to cgroup {path}: {source}")]
    AddProcessFailed {
        pid: u32,
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write cgroup file {path}: {source}")]
    WriteFailed {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to clean up cgroup {path}: {source}")]
    CleanupFailed {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

// ---------------------------------------------------------------------------
// Namespace errors
// ---------------------------------------------------------------------------

/// Errors from Linux namespace operations (clone, setns, etc.).
#[derive(Debug, Error)]
pub enum NamespaceError {
    #[error("clone(2) failed: {0}")]
    CloneFailed(String),

    #[error("failed to set hostname: {0}")]
    SetHostnameFailed(String),

    #[error("namespace error: {0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// Process errors
// ---------------------------------------------------------------------------

/// Errors from spawning and managing container processes.
#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("failed to spawn container process: {0}")]
    SpawnFailed(String),

    #[error("exec failed for command {cmd}: {source}")]
    ExecFailed {
        cmd: String,
        #[source]
        source: nix::errno::Errno,
    },

    #[error("waitpid failed for PID {pid}: {source}")]
    WaitFailed {
        pid: u32,
        #[source]
        source: nix::errno::Errno,
    },

    #[error("process error: {0}")]
    Other(String),
}
