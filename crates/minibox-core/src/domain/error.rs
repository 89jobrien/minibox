//! Infrastructure-independent domain error types.

// ---------------------------------------------------------------------------
// Domain Errors
// ---------------------------------------------------------------------------

/// Domain-specific errors that are independent of infrastructure.
///
/// These errors represent business logic failures, not infrastructure
/// failures. Infrastructure adapters should map their specific errors
/// (e.g., `std::io::Error`, `reqwest::Error`) to these domain errors
/// when appropriate, or return generic `anyhow::Error` for infrastructure
/// failures.
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    /// Image was not found in the registry or local cache.
    #[error("image {name}:{tag} not found")]
    ImageNotFound {
        /// Image name (e.g., `"library/ubuntu"`).
        name: String,
        /// Image tag (e.g., `"22.04"`).
        tag: String,
    },

    /// Image pull from registry failed.
    #[error("failed to pull image '{image}:{tag}': {source}")]
    ImagePullFailed {
        /// Image name.
        image: String,
        /// Image tag.
        tag: String,
        /// Underlying error from the registry adapter.
        #[source]
        source: anyhow::Error,
    },

    /// Image has no layers (corrupted or invalid image).
    #[error("image {name}:{tag} has no layers")]
    EmptyImage {
        /// Image name.
        name: String,
        /// Image tag.
        tag: String,
    },

    /// Container was not found in the daemon state.
    #[error("container '{id}' not found")]
    ContainerNotFound {
        /// Container ID.
        id: String,
    },

    /// Container process failed to spawn.
    #[error("container '{id}' failed to spawn: {source}")]
    ContainerSpawnFailed {
        /// Container ID.
        id: String,
        /// Underlying error from the runtime adapter.
        #[source]
        source: anyhow::Error,
    },

    /// Attempted operation on a running container that requires it to be stopped.
    #[error("container '{id}' is already running")]
    AlreadyRunning {
        /// Container ID.
        id: String,
    },

    /// Attempted to remove a container that is not stopped (running or paused).
    #[error("container '{id}' is not stopped (current state: {state})")]
    ContainerNotStopped {
        /// Container ID.
        id: String,
        /// Current container state (e.g. "Running", "Paused").
        state: String,
    },

    /// Invalid container configuration provided.
    #[error("invalid container configuration: {0}")]
    InvalidConfig(String),

    /// Resource limit values are outside acceptable ranges.
    #[error("invalid resource limits: {0}")]
    InvalidResourceLimits(String),

    /// A resource limit value exceeded the allowed maximum.
    #[error("resource limit '{limit}': value {value} exceeds maximum {max}")]
    ResourceLimitExceeded {
        /// Name of the limit (e.g., `"memory_bytes"`).
        limit: String,
        /// The value that was provided.
        value: u64,
        /// The maximum allowed value.
        max: u64,
    },

    /// An infrastructure error that does not fit a more specific variant.
    #[error(transparent)]
    InfrastructureError(#[from] anyhow::Error),
}

impl DomainError {
    /// Return a short machine-readable identifier for this error variant.
    ///
    /// Useful for metrics labels and structured log fields.
    pub const fn error_kind(&self) -> &'static str {
        match self {
            Self::ImageNotFound { .. } => "image_not_found",
            Self::ImagePullFailed { .. } => "image_pull_failed",
            Self::EmptyImage { .. } => "empty_image",
            Self::ContainerNotFound { .. } => "container_not_found",
            Self::ContainerSpawnFailed { .. } => "container_spawn_failed",
            Self::AlreadyRunning { .. } => "already_running",
            Self::ContainerNotStopped { .. } => "container_not_stopped",
            Self::InvalidConfig(_) => "invalid_config",
            Self::InvalidResourceLimits(_) => "invalid_resource_limits",
            Self::ResourceLimitExceeded { .. } => "resource_limit_exceeded",
            Self::InfrastructureError(_) => "infrastructure_error",
        }
    }
}
