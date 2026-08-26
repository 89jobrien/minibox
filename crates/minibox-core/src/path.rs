//! Path validation primitives that prevent traversal outside trusted roots.

use anyhow::{Context, Result, bail};
use std::path::{Component, Path, PathBuf};

pub use minibox_domain::path::InternalPath;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// A path lexically validated against a canonical trusted base directory.
///
/// Existing paths are canonicalized. New paths retain their joined form, so
/// callers still need race-safe filesystem operations when creating them.
pub struct ValidatedPath {
    inner: PathBuf,
    base: PathBuf,
}

impl ValidatedPath {
    /// Create a validated path relative to `base_dir`.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - `path` is absolute
    /// - `path` contains `..` components
    /// - canonicalization of `base_dir` fails
    /// - the resolved path escapes `base_dir`
    pub fn new(path: &Path, base_dir: &Path) -> Result<Self> {
        if path.is_absolute() {
            bail!(
                "path validation failed: absolute path not allowed: {}",
                path.display()
            );
        }
        if has_parent_component(path) {
            bail!(
                "path validation failed: '..' component not allowed: {}",
                path.display()
            );
        }
        let full = base_dir.join(path);
        let canonical_base = base_dir
            .canonicalize()
            .with_context(|| format!("canonicalize base {}", base_dir.display()))?;
        if let Some(parent) = full.parent() {
            if parent.exists() {
                let canonical = parent.canonicalize()?;
                if !canonical.starts_with(&canonical_base) {
                    bail!(
                        "path validation failed: {} resolves outside base {}",
                        path.display(),
                        base_dir.display()
                    );
                }
            }
        }
        if full.exists() {
            let canonical = full.canonicalize()?;
            if !canonical.starts_with(&canonical_base) {
                bail!(
                    "path validation failed: {} resolves outside base {}",
                    path.display(),
                    base_dir.display()
                );
            }
            // Store the canonical path to eliminate TOCTOU window between
            // validation and use. Callers still need O_NOFOLLOW or
            // openat2(RESOLVE_BENEATH) for race-free access to new paths.
            return Ok(Self {
                inner: canonical,
                base: canonical_base,
            });
        }
        Ok(Self {
            inner: full,
            base: canonical_base,
        })
    }

    #[cfg(test)]
    /// Validates an existing absolute path against a trusted base directory.
    pub fn from_absolute(abs_path: &Path, base_dir: &Path) -> Result<Self> {
        let canonical_base = base_dir
            .canonicalize()
            .with_context(|| format!("canonicalize base {}", base_dir.display()))?;
        let canonical = abs_path
            .canonicalize()
            .with_context(|| format!("canonicalize path {}", abs_path.display()))?;
        if !canonical.starts_with(&canonical_base) {
            bail!(
                "path validation failed: {} is outside base {}",
                abs_path.display(),
                base_dir.display()
            );
        }
        Ok(Self {
            inner: canonical,
            base: canonical_base,
        })
    }

    #[must_use]
    /// Returns the validated path.
    pub fn as_path(&self) -> &Path {
        &self.inner
    }

    #[must_use]
    /// Returns the canonical base directory used for validation.
    pub fn base_dir(&self) -> &Path {
        &self.base
    }

    #[cfg(test)]
    /// Appends and validates a relative component against the original base.
    pub fn join_validated(&self, component: &Path) -> Result<Self> {
        if component.is_absolute() {
            bail!(
                "join_validated: component must be relative: {}",
                component.display()
            );
        }
        if has_parent_component(component) {
            bail!(
                "join_validated: '..' not allowed in component: {}",
                component.display()
            );
        }
        let joined = self.inner.join(component);
        if joined.exists() {
            let canonical = joined.canonicalize()?;
            if !canonical.starts_with(&self.base) {
                bail!(
                    "join_validated: {} escapes base {}",
                    component.display(),
                    self.base.display()
                );
            }
        }
        Ok(Self {
            inner: joined,
            base: self.base.clone(),
        })
    }
}

impl std::fmt::Display for ValidatedPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.display().fmt(f)
    }
}

fn has_parent_component(path: &Path) -> bool {
    path.components().any(|c| matches!(c, Component::ParentDir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn new_rejects_absolute_path() {
        let base = TempDir::new().unwrap();
        let err = ValidatedPath::new(Path::new("/etc/passwd"), base.path()).unwrap_err();
        assert!(
            format!("{err:?}").contains("absolute"),
            "error should mention 'absolute': {err:?}"
        );
    }

    #[test]
    fn new_rejects_parent_traversal() {
        let base = TempDir::new().unwrap();
        let err = ValidatedPath::new(Path::new("../escape"), base.path()).unwrap_err();
        assert!(
            format!("{err:?}").contains(".."),
            "error should mention '..': {err:?}"
        );
    }

    #[test]
    fn new_accepts_valid_relative_path() {
        let base = TempDir::new().unwrap();
        fs::create_dir_all(base.path().join("sub/dir")).unwrap();
        let vp =
            ValidatedPath::new(Path::new("sub/dir"), base.path()).expect("valid relative path");
        assert!(vp.as_path().ends_with("sub/dir"));
        assert_eq!(vp.base_dir(), base.path().canonicalize().unwrap());
    }

    #[test]
    fn new_rejects_symlink_escape() {
        let base = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let link = base.path().join("escape_link");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        let err = ValidatedPath::new(Path::new("escape_link"), base.path()).unwrap_err();
        assert!(
            format!("{err:?}").contains("outside"),
            "error should mention 'outside': {err:?}"
        );
    }

    #[test]
    fn as_path_returns_inner() {
        let base = TempDir::new().unwrap();
        let sub = base.path().join("x");
        fs::create_dir(&sub).unwrap();
        let vp = ValidatedPath::new(Path::new("x"), base.path()).unwrap();
        assert_eq!(
            vp.as_path().canonicalize().unwrap(),
            sub.canonicalize().unwrap(),
        );
    }

    #[test]
    fn from_absolute_validates_containment() {
        let base = TempDir::new().unwrap();
        let inside = base.path().join("ok");
        fs::create_dir(&inside).unwrap();
        let vp = ValidatedPath::from_absolute(&inside, base.path()).unwrap();
        assert!(
            vp.as_path()
                .starts_with(base.path().canonicalize().unwrap())
        );
    }

    #[test]
    fn from_absolute_rejects_outside() {
        let base = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let err = ValidatedPath::from_absolute(outside.path(), base.path()).unwrap_err();
        assert!(format!("{err:?}").contains("outside"));
    }

    #[test]
    fn join_validated_revalidates() {
        let base = TempDir::new().unwrap();
        fs::create_dir_all(base.path().join("a/b")).unwrap();
        let vp = ValidatedPath::new(Path::new("a"), base.path()).unwrap();
        let joined = vp.join_validated(Path::new("b")).unwrap();
        assert!(joined.as_path().ends_with("a/b"));
    }

    #[test]
    fn join_validated_rejects_escape() {
        let base = TempDir::new().unwrap();
        fs::create_dir(base.path().join("a")).unwrap();
        let vp = ValidatedPath::new(Path::new("a"), base.path()).unwrap();
        assert!(vp.join_validated(Path::new("../../etc")).is_err());
    }

    #[test]
    fn display_shows_path() {
        let base = TempDir::new().unwrap();
        fs::create_dir(base.path().join("d")).unwrap();
        let vp = ValidatedPath::new(Path::new("d"), base.path()).unwrap();
        let s = format!("{vp}");
        assert!(s.contains("d"), "display should contain 'd': {s}");
    }

    #[test]
    fn internal_path_deref_to_path() {
        let ip = InternalPath::new(PathBuf::from("/var/lib/minibox"));
        let p: &Path = &ip;
        assert_eq!(p, Path::new("/var/lib/minibox"));
    }

    #[test]
    fn internal_path_display() {
        let ip = InternalPath::new(PathBuf::from("/tmp/merged"));
        assert_eq!(format!("{ip}"), "/tmp/merged");
    }

    #[test]
    fn internal_path_from_pathbuf() {
        let ip = InternalPath::from(PathBuf::from("/x"));
        assert_eq!(ip.as_ref(), Path::new("/x"));
    }

    #[test]
    fn internal_path_into_pathbuf() {
        let ip = InternalPath::new(PathBuf::from("/y"));
        let pb: PathBuf = ip.into_inner();
        assert_eq!(pb, PathBuf::from("/y"));
    }

    mod proptest_validated {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn valid_path_is_within_base(name in "[a-z]{1,8}") {
                let base = TempDir::new().unwrap();
                std::fs::create_dir(base.path().join(&name)).unwrap();
                let vp = ValidatedPath::new(
                    Path::new(&name), base.path(),
                ).unwrap();
                let canonical_base = base.path().canonicalize().unwrap();
                let canonical_path = vp.as_path().canonicalize().unwrap();
                prop_assert!(canonical_path.starts_with(&canonical_base));
            }

            #[test]
            fn traversal_always_rejected(
                prefix in "[a-z]{0,4}",
                suffix in "[a-z]{1,4}"
            ) {
                let base = TempDir::new().unwrap();
                let evil = format!("{prefix}/../{suffix}");
                let result = ValidatedPath::new(
                    Path::new(&evil), base.path(),
                );
                prop_assert!(result.is_err());
            }
        }
    }
}
