//! Error types for OCI image operations.

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
