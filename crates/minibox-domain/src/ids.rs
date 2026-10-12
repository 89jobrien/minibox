//! Validated identifiers for containers and interactive sessions.

/// Container identifier type.
///
/// Provides type safety for container IDs throughout the domain. The invariant
/// (non-empty, ASCII alphanumeric, at most 64 characters) is enforced by
/// [`nutype`], so it holds on the construction path *and* on the serde
/// deserialization path.
#[nutype::nutype(
    validate(
        not_empty,
        len_char_max = 64,
        predicate = |id: &str| id.chars().all(|c| c.is_ascii_alphanumeric()),
    ),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Serialize, Deserialize),
)]
pub struct ContainerId(String);

impl ContainerId {
    /// Get the ID as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.as_ref()
    }
}

impl std::fmt::Display for ContainerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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

#[cfg(test)]
mod tests {
    use super::ContainerId;

    /// The whole point of routing `ContainerId` through `nutype`: the invariant
    /// holds on the *deserialization* path too, not just `try_new`.
    #[test]
    fn deserialize_rejects_invalid_ids() {
        for bad in ["", "abc-123", "abc 123", &"a".repeat(65)] {
            assert!(
                serde_json::from_str::<ContainerId>(&format!("\"{bad}\"")).is_err(),
                "deserialization must reject {bad:?}"
            );
        }
    }

    #[test]
    fn deserialize_accepts_valid_id() {
        let id: ContainerId = serde_json::from_str("\"abc123\"").expect("valid id");
        assert_eq!(id.as_str(), "abc123");
    }

    /// Round-trips through serde without loss.
    #[test]
    fn roundtrips_through_serde() {
        let id = ContainerId::try_new("deadbeef".to_string()).expect("valid id");
        let json = serde_json::to_string(&id).expect("serializes");
        let back: ContainerId = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(id, back);
    }

    /// The invariant is still enforced on the construction path.
    #[test]
    fn try_new_enforces_invariant() {
        assert!(ContainerId::try_new("abc123".to_string()).is_ok());
        assert!(ContainerId::try_new(String::new()).is_err());
        assert!(ContainerId::try_new("abc-123".to_string()).is_err());
        assert!(ContainerId::try_new("a".repeat(65)).is_err());
    }
}
