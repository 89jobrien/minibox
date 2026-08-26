//! Validated identifiers for containers and interactive sessions.

use anyhow::Result;

/// Container identifier type.
///
/// Provides type safety for container IDs throughout the domain.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContainerId(String);

impl ContainerId {
    /// Create a new container ID.
    ///
    /// # Validation
    ///
    /// IDs must be:
    /// - Non-empty
    /// - Alphanumeric (a-z, A-Z, 0-9)
    /// - Between 1 and 64 characters
    ///
    /// # Errors
    ///
    /// Returns an error if any validation rule is violated.
    pub fn new(id: String) -> Result<Self> {
        const MAX_CONTAINER_ID_LEN: usize = 64;
        if id.is_empty() {
            anyhow::bail!("container ID cannot be empty");
        }
        if id.len() > MAX_CONTAINER_ID_LEN {
            anyhow::bail!(
                "container ID too long: {} (max {MAX_CONTAINER_ID_LEN})",
                id.len()
            );
        }
        if !id.chars().all(|c| c.is_ascii_alphanumeric()) {
            anyhow::bail!("container ID must be alphanumeric");
        }
        Ok(Self(id))
    }

    /// Get the ID as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ContainerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ContainerId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// ---------------------------------------------------------------------------
// Session ID — PTY / interactive exec sessions
// ---------------------------------------------------------------------------

/// Opaque identifier for a live PTY or interactive exec session.
///
/// Parallel to [`ContainerId`] but scoped to exec sessions rather than
/// container lifecycle. A session is created when `Exec` or `Run` is invoked
/// with `tty: true` and destroyed when the process exits.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SessionId(String);

impl SessionId {
    /// Create a new session ID from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the ID as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for SessionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for SessionId {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SessionId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

// ---------------------------------------------------------------------------
