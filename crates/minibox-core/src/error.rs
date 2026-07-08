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

    #[error("layer download task panicked or was cancelled for digest {digest}: {source}")]
    #[diagnostic(code(minibox::registry::layer_task))]
    LayerTask {
        digest: String,
        #[source]
        source: tokio::task::JoinError,
    },

    #[error("registry error: {0}")]
    #[diagnostic(code(minibox::registry::other))]
    Other(String),
}

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
    ContainerNotRunning { container_id: String },

    #[error("exec {exec_id} not found")]
    #[diagnostic(code(minibox::exec::not_found))]
    ExecNotFound { exec_id: String },

    #[error("nsenter failed for container {container_id}: {reason}")]
    #[diagnostic(code(minibox::exec::nsenter_failed))]
    NsenterFailed {
        container_id: String,
        reason: String,
    },

    #[error("io error: {0}")]
    #[diagnostic(code(minibox::exec::io))]
    Io(#[from] std::io::Error),

    #[error("exec error: {0}")]
    #[diagnostic(code(minibox::exec::other))]
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
    UpperdirMissing { container_id: String },

    #[error("layer tar failed: {reason}")]
    #[diagnostic(code(minibox::commit::layer_tar))]
    LayerTarFailed { reason: String },

    #[error("io error: {0}")]
    #[diagnostic(code(minibox::commit::io))]
    Io(#[from] std::io::Error),

    #[error("commit error: {0}")]
    #[diagnostic(code(minibox::commit::other))]
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
    DockerfileNotFound { path: String },

    #[error("parse error at line {line}: {reason}")]
    #[diagnostic(code(minibox::build::parse_error))]
    ParseError { line: u32, reason: String },

    #[error("unsupported instruction: {instruction}")]
    #[diagnostic(
        code(minibox::build::unsupported),
        help(
            "minibox supports: FROM, RUN, COPY, ADD, ENV, WORKDIR, CMD, ENTRYPOINT, EXPOSE, LABEL, ARG, USER"
        )
    )]
    UnsupportedInstruction { instruction: String },

    #[error("build step {step} failed with exit code {exit_code}")]
    #[diagnostic(code(minibox::build::step_failed))]
    BuildStepFailed { step: u32, exit_code: i32 },

    #[error("io error: {0}")]
    #[diagnostic(code(minibox::build::io))]
    Io(#[from] std::io::Error),

    #[error("build error: {0}")]
    #[diagnostic(code(minibox::build::other))]
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
    AuthFailed { registry: String, message: String },

    #[error("blob upload failed for {digest}: {reason}")]
    #[diagnostic(code(minibox::push::blob_upload))]
    BlobUploadFailed { digest: String, reason: String },

    #[error("manifest push failed: {reason}")]
    #[diagnostic(code(minibox::push::manifest))]
    ManifestPushFailed { reason: String },

    #[error("network error: {0}")]
    #[diagnostic(code(minibox::push::network))]
    Network(#[from] reqwest::Error),

    #[error("push error: {0}")]
    #[diagnostic(code(minibox::push::other))]
    Other(String),
}
