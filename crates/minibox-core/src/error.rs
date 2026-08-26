//! Shared and module-specific error types for minibox.
//!
//! Each sub-module has its own fine-grained error type; they all implement
//! `std::error::Error` via thiserror and [`miette::Diagnostic`] for rich
//! CLI rendering (error codes, help text, related errors).
//!
//! This module contains only cross-platform error types. Linux-specific errors
//! that depend on `nix` (`FilesystemError`, `CgroupError`, `NamespaceError`,
//! `ProcessError`) remain in the `minibox` crate.

use miette::Diagnostic;
use thiserror::Error;

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
    /// The requested image is absent from the local store.
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
    /// A downloaded or extracted layer failed digest verification.
    DigestMismatch {
        /// Digest identifying the affected layer.
        digest: String,
        /// Expected digest value.
        expected: String,
        /// Computed digest value.
        actual: String,
    },

    #[error("layer extraction failed: {0}")]
    #[diagnostic(code(minibox::image::layer_extract))]
    /// A layer archive could not be extracted.
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
    /// An image operation encountered an I/O error.
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
        /// Registry-provided or local failure detail.
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
    /// A manifest list contains no entry for the requested platform.
    NoPlatformManifest {
        /// Requested operating system and architecture.
        platform: String,
    },

    #[error("manifest list nesting too deep (max 2 levels)")]
    #[diagnostic(code(minibox::registry::nesting_too_deep))]
    /// Manifest-list recursion exceeded the supported depth.
    ManifestNestingTooDeep,

    #[error("layer download task panicked or was cancelled for digest {digest}: {source}")]
    #[diagnostic(code(minibox::registry::layer_task))]
    /// An asynchronous layer download task failed to complete.
    LayerTask {
        /// Layer digest assigned to the task.
        digest: String,
        /// Task join failure.
        #[source]
        source: tokio::task::JoinError,
    },

    #[error("manifest too large: {size} bytes (max {max} bytes)")]
    #[diagnostic(
        code(minibox::registry::manifest_too_large),
        help(
            "the registry returned a manifest that exceeds the safety limit; this may indicate a malformed or adversarial response"
        )
    )]
    /// A registry manifest exceeded the configured safety limit.
    ManifestTooLarge {
        /// Received manifest size in bytes.
        size: u64,
        /// Maximum permitted size in bytes.
        max: u64,
    },

    #[error("layer too large: {size} bytes (max {max} bytes)")]
    #[diagnostic(
        code(minibox::registry::layer_too_large),
        help(
            "the layer blob exceeds the maximum allowed size; use a smaller base image or increase MAX_LAYER_SIZE if intentional"
        )
    )]
    /// A registry layer exceeded the configured safety limit.
    LayerTooLarge {
        /// Received layer size in bytes.
        size: u64,
        /// Maximum permitted size in bytes.
        max: u64,
    },

    #[error("registry error: {0}")]
    #[diagnostic(code(minibox::registry::other))]
    /// A registry operation failed for another reason.
    Other(String),
}

// ---------------------------------------------------------------------------
// Top-level error
// ---------------------------------------------------------------------------

#[derive(Debug, Error, Diagnostic)]
/// Top-level errors shared by minibox applications.
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
    /// An uncategorized application error occurred.
    Other(#[from] anyhow::Error),
}

// ---------------------------------------------------------------------------
// Exec errors
// ---------------------------------------------------------------------------

/// Errors from exec-into-container operations.
#[derive(Debug, Error, Diagnostic)]
pub enum ExecError {
    #[error("container {container_id} is not running")]
    #[diagnostic(
        code(minibox::exec::not_running),
        help("start the container first with: mbx run")
    )]
    /// The target container is not running.
    ContainerNotRunning {
        /// Target container identifier.
        container_id: String,
    },

    #[error("exec {exec_id} not found")]
    #[diagnostic(code(minibox::exec::not_found))]
    /// The requested exec instance was not found.
    ExecNotFound {
        /// Missing exec identifier.
        exec_id: String,
    },

    #[error("nsenter failed for container {container_id}: {reason}")]
    #[diagnostic(code(minibox::exec::nsenter_failed))]
    /// Entering the container namespaces failed.
    NsenterFailed {
        /// Target container identifier.
        container_id: String,
        /// Namespace-entry failure detail.
        reason: String,
    },

    #[error("io error: {0}")]
    #[diagnostic(code(minibox::exec::io))]
    /// An exec I/O operation failed.
    Io(#[from] std::io::Error),

    #[error("exec error: {0}")]
    #[diagnostic(code(minibox::exec::other))]
    /// An exec operation failed for another reason.
    Other(String),
}

// ---------------------------------------------------------------------------
// Push errors
// ---------------------------------------------------------------------------

/// Errors from container commit operations.
#[derive(Debug, Error, Diagnostic)]
pub enum CommitError {
    #[error("overlay upperdir missing for container {container_id}")]
    #[diagnostic(code(minibox::commit::upperdir_missing))]
    /// The container has no recorded writable overlay directory.
    UpperdirMissing {
        /// Container missing writable-layer metadata.
        container_id: String,
    },

    #[error("layer tar failed: {reason}")]
    #[diagnostic(code(minibox::commit::layer_tar))]
    /// Packaging the writable layer as an archive failed.
    LayerTarFailed {
        /// Archive failure detail.
        reason: String,
    },

    #[error("io error: {0}")]
    #[diagnostic(code(minibox::commit::io))]
    /// A commit I/O operation failed.
    Io(#[from] std::io::Error),

    #[error("commit error: {0}")]
    #[diagnostic(code(minibox::commit::other))]
    /// A commit operation failed for another reason.
    Other(String),
}

// ---------------------------------------------------------------------------
// Build errors
// ---------------------------------------------------------------------------

/// Errors from Dockerfile build operations.
#[derive(Debug, Error, Diagnostic)]
pub enum BuildError {
    #[error("Dockerfile not found at {path}")]
    #[diagnostic(
        code(minibox::build::dockerfile_not_found),
        help("check the path and ensure the Dockerfile exists")
    )]
    /// The requested Dockerfile does not exist.
    DockerfileNotFound {
        /// Missing Dockerfile path.
        path: String,
    },

    #[error("parse error at line {line}: {reason}")]
    #[diagnostic(code(minibox::build::parse_error))]
    /// A Dockerfile instruction could not be parsed.
    ParseError {
        /// One-based source line number.
        line: u32,
        /// Parse failure detail.
        reason: String,
    },

    #[error("unsupported instruction: {instruction}")]
    #[diagnostic(
        code(minibox::build::unsupported),
        help(
            "minibox supports: FROM, RUN, COPY, ADD, ENV, WORKDIR, CMD, ENTRYPOINT, EXPOSE, LABEL, ARG, USER"
        )
    )]
    /// The Dockerfile uses an unsupported instruction.
    UnsupportedInstruction {
        /// Unsupported instruction name.
        instruction: String,
    },

    #[error("build step {step} failed with exit code {exit_code}")]
    #[diagnostic(code(minibox::build::step_failed))]
    /// A build command exited unsuccessfully.
    BuildStepFailed {
        /// One-based build step number.
        step: u32,
        /// Process exit code.
        exit_code: i32,
    },

    #[error("io error: {0}")]
    #[diagnostic(code(minibox::build::io))]
    /// A build I/O operation failed.
    Io(#[from] std::io::Error),

    #[error("build error: {0}")]
    #[diagnostic(code(minibox::build::other))]
    /// A build operation failed for another reason.
    Other(String),
}

/// Errors from OCI image push operations.
#[derive(Debug, Error, Diagnostic)]
pub enum PushError {
    #[error("registry authentication failed for {registry}: {message}")]
    #[diagnostic(
        code(minibox::push::auth_failed),
        help("verify registry credentials and permissions")
    )]
    /// Registry authentication failed during a push.
    AuthFailed {
        /// Target registry hostname.
        registry: String,
        /// Authentication failure detail.
        message: String,
    },

    #[error("blob upload failed for {digest}: {reason}")]
    #[diagnostic(code(minibox::push::blob_upload))]
    /// Uploading an image blob failed.
    BlobUploadFailed {
        /// Blob digest.
        digest: String,
        /// Upload failure detail.
        reason: String,
    },

    #[error("manifest push failed: {reason}")]
    #[diagnostic(code(minibox::push::manifest))]
    /// Uploading the image manifest failed.
    ManifestPushFailed {
        /// Push failure detail.
        reason: String,
    },

    #[cfg(feature = "registry")]
    #[error("network error: {0}")]
    #[diagnostic(code(minibox::push::network))]
    /// A push HTTP request failed.
    Network(#[from] reqwest::Error),

    #[error("push error: {0}")]
    #[diagnostic(code(minibox::push::other))]
    /// A push operation failed for another reason.
    Other(String),
}
