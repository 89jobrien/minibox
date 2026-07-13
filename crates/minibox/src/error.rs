//! Shared and module-specific error types for minibox.
//!
//! Each sub-module has its own fine-grained error type; they all implement
//! `std::error::Error` via thiserror and [`miette::Diagnostic`] for rich
//! CLI rendering (error codes, help text, related errors).

use miette::Diagnostic;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Top-level error
// ---------------------------------------------------------------------------

#[derive(Debug, Error, Diagnostic)]
pub enum MiniboxError {
    #[error("image not found: {0}")]
    #[diagnostic(
        code(minibox::image_not_found),
        help("pull the image first with: mbx pull <image>")
    )]
    ImageNotFound(String),

    #[error("container not found: {id}")]
    #[diagnostic(
        code(minibox::container_not_found),
        help("run 'mbx ps' to list containers")
    )]
    ContainerNotFound { id: String },

    #[error("container is not in the expected state: {0}")]
    #[diagnostic(code(minibox::invalid_state))]
    InvalidState(String),

    #[error("io error: {0}")]
    #[diagnostic(code(minibox::io))]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    #[diagnostic(code(minibox::json))]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

// ---------------------------------------------------------------------------
// Filesystem errors
// ---------------------------------------------------------------------------

/// Errors from filesystem / overlay / mount operations.
#[derive(Debug, Error, Diagnostic)]
pub enum FilesystemError {
    #[error("failed to create directory {path}: {source}")]
    #[diagnostic(code(minibox::fs::create_dir))]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to mount {fs} at {target}: {source}")]
    #[diagnostic(code(minibox::fs::mount))]
    Mount {
        fs: String,
        target: String,
        #[source]
        source: nix::errno::Errno,
    },

    #[error("overlay mount failed: {0}")]
    #[diagnostic(
        code(minibox::fs::overlay),
        help("ensure overlayfs is supported and the layer paths exist")
    )]
    OverlayMount(String),

    #[error("pivot_root failed: {0}")]
    #[diagnostic(code(minibox::fs::pivot_root))]
    PivotRoot(String),

    #[error("failed to unmount {target}: {source}")]
    #[diagnostic(code(minibox::fs::umount))]
    Umount {
        target: String,
        #[source]
        source: nix::errno::Errno,
    },

    #[error("cleanup failed for {path}: {source}")]
    #[diagnostic(code(minibox::fs::cleanup))]
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
#[derive(Debug, Error, Diagnostic)]
pub enum ImageError {
    #[error("image {name}:{tag} not found in local store")]
    #[diagnostic(
        code(minibox::image::not_found),
        help("pull the image first: mbx pull {name}")
    )]
    NotFound { name: String, tag: String },

    #[error("digest mismatch for {digest}: expected {expected}, got {actual}")]
    #[diagnostic(
        code(minibox::image::digest_mismatch),
        help("the layer may be corrupted; re-pull the image")
    )]
    DigestMismatch {
        digest: String,
        expected: String,
        actual: String,
    },

    #[error("layer extraction failed: {0}")]
    #[diagnostic(code(minibox::image::layer_extract))]
    LayerExtract(String),

    /// A tar entry was a block or character device node, which is rejected for
    /// security reasons.  The `entry` field is the path of the offending entry.
    #[error("tar entry is a device node (security rejected): {entry}")]
    #[diagnostic(
        code(minibox::image::device_node_rejected),
        help("device nodes are not allowed in container images for security reasons")
    )]
    DeviceNodeRejected { entry: String },

    /// A tar entry's symlink target contained `..` components after the
    /// absolute->relative rewrite, which would escape the container root.
    #[error(
        "tar entry symlink traverses parent directory (security rejected): {entry} -> {target}"
    )]
    #[diagnostic(
        code(minibox::image::symlink_traversal),
        help("symlinks must not escape the container root via '..' components")
    )]
    SymlinkTraversalRejected { entry: String, target: String },

    #[error("failed to write to image store at {path}: {source}")]
    #[diagnostic(code(minibox::image::store_write))]
    StoreWrite {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read from image store at {path}: {source}")]
    #[diagnostic(code(minibox::image::store_read))]
    StoreRead {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse manifest for {name}:{tag}: {source}")]
    #[diagnostic(code(minibox::image::manifest_parse))]
    ManifestParse {
        name: String,
        tag: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("io error: {0}")]
    #[diagnostic(code(minibox::image::io))]
    Io(#[from] std::io::Error),

    #[error("layer error: {0}")]
    #[diagnostic(code(minibox::image::other))]
    Other(String),
}

// ---------------------------------------------------------------------------
// Registry errors
// ---------------------------------------------------------------------------

/// Errors from the OCI registry client.
#[derive(Debug, Error, Diagnostic)]
pub enum RegistryError {
    #[error("network error: {0}")]
    #[diagnostic(
        code(minibox::registry::network),
        help("check your network connection and DNS resolution")
    )]
    Network(#[from] reqwest::Error),

    #[error("authentication failed for {image}: {message}")]
    #[diagnostic(
        code(minibox::registry::auth_failed),
        help("verify registry credentials; for Docker Hub, check rate limits")
    )]
    AuthFailed { image: String, message: String },

    #[error("failed to fetch manifest for {name}:{tag}: {message}")]
    #[diagnostic(code(minibox::registry::manifest_fetch))]
    ManifestFetch {
        name: String,
        tag: String,
        message: String,
    },

    #[error("failed to fetch blob {digest}: {message}")]
    #[diagnostic(code(minibox::registry::blob_fetch))]
    BlobFetch { digest: String, message: String },

    #[error("no {platform} manifest found in manifest list")]
    #[diagnostic(
        code(minibox::registry::no_platform),
        help("use --platform to specify a different architecture")
    )]
    NoPlatformManifest { platform: String },

    #[error("manifest list nesting too deep (max 2 levels)")]
    #[diagnostic(code(minibox::registry::nesting_too_deep))]
    ManifestNestingTooDeep,

    #[error("registry error: {0}")]
    #[diagnostic(code(minibox::registry::other))]
    Other(String),
}

// ---------------------------------------------------------------------------
// Cgroup errors
// ---------------------------------------------------------------------------

/// Errors from cgroup v2 operations.
#[derive(Debug, Error, Diagnostic)]
pub enum CgroupError {
    #[error("failed to create cgroup directory {path}: {source}")]
    #[diagnostic(code(minibox::cgroup::create))]
    CreateFailed {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to add process {pid} to cgroup {path}: {source}")]
    #[diagnostic(code(minibox::cgroup::add_process))]
    AddProcessFailed {
        pid: u32,
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write cgroup file {path}: {source}")]
    #[diagnostic(code(minibox::cgroup::write))]
    WriteFailed {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to clean up cgroup {path}: {source}")]
    #[diagnostic(code(minibox::cgroup::cleanup))]
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
#[derive(Debug, Error, Diagnostic)]
pub enum NamespaceError {
    #[error("clone(2) failed: {0}")]
    #[diagnostic(
        code(minibox::namespace::clone_failed),
        help("ensure you have CAP_SYS_ADMIN or are running as root")
    )]
    CloneFailed(String),

    #[error("failed to set hostname: {0}")]
    #[diagnostic(code(minibox::namespace::hostname))]
    SetHostnameFailed(String),

    #[error("namespace error: {0}")]
    #[diagnostic(code(minibox::namespace::other))]
    Other(String),
}

// ---------------------------------------------------------------------------
// Process errors
// ---------------------------------------------------------------------------

/// Errors from spawning and managing container processes.
#[derive(Debug, Error, Diagnostic)]
pub enum ProcessError {
    #[error("failed to spawn container process: {0}")]
    #[diagnostic(code(minibox::process::spawn))]
    SpawnFailed(String),

    #[error("exec failed for command {cmd}: {source}")]
    #[diagnostic(code(minibox::process::exec))]
    ExecFailed {
        cmd: String,
        #[source]
        source: nix::errno::Errno,
    },

    #[error("waitpid failed for PID {pid}: {source}")]
    #[diagnostic(code(minibox::process::wait))]
    WaitFailed {
        pid: u32,
        #[source]
        source: nix::errno::Errno,
    },

    #[error("process error: {0}")]
    #[diagnostic(code(minibox::process::other))]
    Other(String),
}
