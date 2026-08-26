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
/// Top-level errors returned by the native runtime crate.
pub enum MiniboxError {
    #[error("image not found: {0}")]
    #[diagnostic(
        code(minibox::image_not_found),
        help("pull the image first with: mbx pull <image>")
    )]
    /// The requested image was not found.
    ImageNotFound(String),

    #[error("container not found: {id}")]
    #[diagnostic(
        code(minibox::container_not_found),
        help("run 'mbx ps' to list containers")
    )]
    /// The requested container was not found.
    ContainerNotFound {
        /// Missing container identifier.
        id: String,
    },

    #[error("container is not in the expected state: {0}")]
    #[diagnostic(code(minibox::invalid_state))]
    /// A container was in an invalid state for the operation.
    InvalidState(String),

    #[error("io error: {0}")]
    #[diagnostic(code(minibox::io))]
    /// An underlying I/O operation failed.
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    #[diagnostic(code(minibox::json))]
    /// JSON encoding or decoding failed.
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    /// An uncategorized runtime error occurred.
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
    /// A required directory could not be created.
    CreateDir {
        /// Directory path.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    #[error("failed to mount {fs} at {target}: {source}")]
    #[diagnostic(code(minibox::fs::mount))]
    /// A filesystem could not be mounted.
    Mount {
        /// Filesystem source or type.
        fs: String,
        /// Mount target path.
        target: String,
        /// Kernel mount error.
        #[source]
        source: nix::errno::Errno,
    },

    #[error("overlay mount failed: {0}")]
    #[diagnostic(
        code(minibox::fs::overlay),
        help("ensure overlayfs is supported and the layer paths exist")
    )]
    /// Mounting an overlay filesystem failed.
    OverlayMount(String),

    #[error("pivot_root failed: {0}")]
    #[diagnostic(code(minibox::fs::pivot_root))]
    /// Switching to the container root filesystem failed.
    PivotRoot(String),

    #[error("failed to unmount {target}: {source}")]
    #[diagnostic(code(minibox::fs::umount))]
    /// A filesystem could not be unmounted.
    Umount {
        /// Mount target path.
        target: String,
        /// Kernel unmount error.
        #[source]
        source: nix::errno::Errno,
    },

    #[error("cleanup failed for {path}: {source}")]
    #[diagnostic(code(minibox::fs::cleanup))]
    /// Filesystem cleanup failed.
    Cleanup {
        /// Path being cleaned.
        path: String,
        /// Underlying I/O error.
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
    /// The requested image is absent from local storage.
    NotFound {
        /// Image repository name.
        name: String,
        /// Image tag.
        tag: String,
    },

    #[error("digest mismatch for {digest}: expected {expected}, got {actual}")]
    #[diagnostic(
        code(minibox::image::digest_mismatch),
        help("the layer may be corrupted; re-pull the image")
    )]
    /// A layer failed digest verification.
    DigestMismatch {
        /// Layer digest identifier.
        digest: String,
        /// Expected digest value.
        expected: String,
        /// Computed digest value.
        actual: String,
    },

    #[error("layer extraction failed: {0}")]
    #[diagnostic(code(minibox::image::layer_extract))]
    /// Extracting a layer archive failed.
    LayerExtract(String),

    /// A tar entry was a block or character device node, which is rejected for
    /// security reasons.  The `entry` field is the path of the offending entry.
    #[error("tar entry is a device node (security rejected): {entry}")]
    #[diagnostic(
        code(minibox::image::device_node_rejected),
        help("device nodes are not allowed in container images for security reasons")
    )]
    DeviceNodeRejected {
        /// Path of the rejected archive entry.
        entry: String,
    },

    /// A tar entry's symlink target contained `..` components after the
    /// absolute->relative rewrite, which would escape the container root.
    #[error(
        "tar entry symlink traverses parent directory (security rejected): {entry} -> {target}"
    )]
    #[diagnostic(
        code(minibox::image::symlink_traversal),
        help("symlinks must not escape the container root via '..' components")
    )]
    SymlinkTraversalRejected {
        /// Path of the rejected symlink entry.
        entry: String,
        /// Unsafe symlink target.
        target: String,
    },

    #[error("failed to write to image store at {path}: {source}")]
    #[diagnostic(code(minibox::image::store_write))]
    /// The image store could not write data.
    StoreWrite {
        /// Store path that could not be written.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read from image store at {path}: {source}")]
    #[diagnostic(code(minibox::image::store_read))]
    /// The image store could not read data.
    StoreRead {
        /// Store path that could not be read.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse manifest for {name}:{tag}: {source}")]
    #[diagnostic(code(minibox::image::manifest_parse))]
    /// A stored image manifest contained invalid JSON.
    ManifestParse {
        /// Image repository name.
        name: String,
        /// Image tag.
        tag: String,
        /// Underlying JSON error.
        #[source]
        source: serde_json::Error,
    },

    #[error("io error: {0}")]
    #[diagnostic(code(minibox::image::io))]
    /// An image I/O operation failed.
    Io(#[from] std::io::Error),

    #[error("layer error: {0}")]
    #[diagnostic(code(minibox::image::other))]
    /// An image operation failed for another reason.
    Other(String),
}

// ---------------------------------------------------------------------------
// Registry errors
// ---------------------------------------------------------------------------

/// Errors from the OCI registry client.
#[derive(Debug, Error, Diagnostic)]
pub enum RegistryError {
    #[cfg(feature = "registry")]
    #[error("network error: {0}")]
    #[diagnostic(
        code(minibox::registry::network),
        help("check your network connection and DNS resolution")
    )]
    /// A registry HTTP request failed.
    Network(#[from] reqwest::Error),

    #[error("authentication failed for {image}: {message}")]
    #[diagnostic(
        code(minibox::registry::auth_failed),
        help("verify registry credentials; for Docker Hub, check rate limits")
    )]
    /// Registry authentication failed.
    AuthFailed {
        /// Image reference being accessed.
        image: String,
        /// Authentication failure detail.
        message: String,
    },

    #[error("failed to fetch manifest for {name}:{tag}: {message}")]
    #[diagnostic(code(minibox::registry::manifest_fetch))]
    /// An image manifest could not be fetched.
    ManifestFetch {
        /// Image repository name.
        name: String,
        /// Image tag.
        tag: String,
        /// Registry response or transport detail.
        message: String,
    },

    #[error("failed to fetch blob {digest}: {message}")]
    #[diagnostic(code(minibox::registry::blob_fetch))]
    /// A registry blob could not be fetched.
    BlobFetch {
        /// Blob digest.
        digest: String,
        /// Registry response or transport detail.
        message: String,
    },

    #[error("no {platform} manifest found in manifest list")]
    #[diagnostic(
        code(minibox::registry::no_platform),
        help("use --platform to specify a different architecture")
    )]
    /// No manifest matched the requested platform.
    NoPlatformManifest {
        /// Requested operating system and architecture.
        platform: String,
    },

    #[error("manifest list nesting too deep (max 2 levels)")]
    #[diagnostic(code(minibox::registry::nesting_too_deep))]
    /// Manifest-list recursion exceeded the supported depth.
    ManifestNestingTooDeep,

    #[error("registry error: {0}")]
    #[diagnostic(code(minibox::registry::other))]
    /// A registry operation failed for another reason.
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
    /// A cgroup directory could not be created.
    CreateFailed {
        /// Cgroup path.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    #[error("failed to add process {pid} to cgroup {path}: {source}")]
    #[diagnostic(code(minibox::cgroup::add_process))]
    /// A process could not be attached to a cgroup.
    AddProcessFailed {
        /// Host process identifier.
        pid: u32,
        /// Cgroup path.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write cgroup file {path}: {source}")]
    #[diagnostic(code(minibox::cgroup::write))]
    /// A cgroup control file could not be written.
    WriteFailed {
        /// Control-file path.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    #[error("failed to clean up cgroup {path}: {source}")]
    #[diagnostic(code(minibox::cgroup::cleanup))]
    /// A cgroup could not be removed.
    CleanupFailed {
        /// Cgroup path.
        path: String,
        /// Underlying I/O error.
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
    /// Creating the namespaced process failed.
    CloneFailed(String),

    #[error("failed to set hostname: {0}")]
    #[diagnostic(code(minibox::namespace::hostname))]
    /// Setting the container hostname failed.
    SetHostnameFailed(String),

    #[error("namespace error: {0}")]
    #[diagnostic(code(minibox::namespace::other))]
    /// A namespace operation failed for another reason.
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
    /// Spawning the container process failed.
    SpawnFailed(String),

    #[error("exec failed for command {cmd}: {source}")]
    #[diagnostic(code(minibox::process::exec))]
    /// Replacing the child process image failed.
    ExecFailed {
        /// Command being executed.
        cmd: String,
        /// Kernel exec error.
        #[source]
        source: nix::errno::Errno,
    },

    #[error("waitpid failed for PID {pid}: {source}")]
    #[diagnostic(code(minibox::process::wait))]
    /// Waiting for a container process failed.
    WaitFailed {
        /// Host process identifier.
        pid: u32,
        /// Kernel wait error.
        #[source]
        source: nix::errno::Errno,
    },

    #[error("process error: {0}")]
    #[diagnostic(code(minibox::process::other))]
    /// A process operation failed for another reason.
    Other(String),

    #[error("failed to install mount immutability seccomp filter: {0}")]
    #[diagnostic(code(minibox::process::seccomp_install))]
    /// Installing the mount-protection seccomp filter failed.
    SeccompInstallFailed(String),
}
