//! Trusted path value types shared by domain contracts.

use std::path::{Path, PathBuf};

/// A daemon-internal path constructed by trusted runtime code.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct InternalPath(PathBuf);

impl InternalPath {
    /// Wrap a trusted daemon-internal path.
    #[must_use]
    pub const fn new(path: PathBuf) -> Self {
        Self(path)
    }

    /// Consume the wrapper and return the underlying path.
    #[must_use]
    pub fn into_inner(self) -> PathBuf {
        self.0
    }
}

impl std::ops::Deref for InternalPath {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for InternalPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl From<PathBuf> for InternalPath {
    fn from(path: PathBuf) -> Self {
        Self(path)
    }
}

impl From<&str> for InternalPath {
    fn from(path: &str) -> Self {
        Self(PathBuf::from(path))
    }
}

impl std::fmt::Display for InternalPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.display().fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_path_roundtrips_path_buf() {
        let path = PathBuf::from("/var/lib/minibox/container");
        let internal = InternalPath::from(path.clone());

        assert_eq!(internal.as_ref(), path.as_path());
        assert_eq!(internal.into_inner(), path);
    }
}
