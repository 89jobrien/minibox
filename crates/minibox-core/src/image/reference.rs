//! Parsing and canonicalization of OCI image references.

use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Parsed OCI image reference with explicit registry, namespace, name, and tag.
pub struct ImageRef {
    /// Registry hostname as supplied or inferred.
    pub registry: String,
    /// Repository namespace or organization.
    pub namespace: String,
    /// Image repository name.
    pub name: String,
    /// Image tag, defaulting to `latest` when omitted.
    pub tag: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
/// Errors produced while parsing an image reference.
pub enum ImageRefError {
    /// The reference was empty.
    #[error("empty image reference")]
    Empty,
    /// The reference had an unsupported or incomplete shape.
    #[error("invalid image reference: {0}")]
    Invalid(String),
}

impl ImageRef {
    /// Parse an image reference string into an [`ImageRef`].
    ///
    /// # Errors
    ///
    /// Returns [`ImageRefError::Empty`] for empty input or [`ImageRefError::Invalid`]
    /// if the reference cannot be parsed.
    pub fn parse(s: &str) -> Result<Self, ImageRefError> {
        if s.is_empty() {
            return Err(ImageRefError::Empty);
        }

        let (path_part, tag) = match s.rsplit_once(':') {
            Some((p, t)) if !t.is_empty() && !t.contains('/') => (p, t.to_owned()),
            _ => (s, "latest".to_owned()),
        };

        let (registry, rest) = match path_part.split_once('/') {
            Some((first, rest))
                if first.contains('.') || first.contains(':') || first == "localhost" =>
            {
                (first.to_owned(), rest)
            }
            _ => ("docker.io".to_owned(), path_part),
        };

        let (namespace, name) = match rest.rsplit_once('/') {
            Some((ns, n)) => (ns.to_owned(), n.to_owned()),
            None => {
                if registry == "docker.io" {
                    ("library".to_owned(), rest.to_owned())
                } else {
                    return Err(ImageRefError::Invalid(format!(
                        "non-docker.io registry requires org/name format, got: {s}"
                    )));
                }
            }
        };

        if name.is_empty() {
            return Err(ImageRefError::Invalid(format!("empty image name in: {s}")));
        }

        Ok(Self {
            registry,
            namespace,
            name,
            tag,
        })
    }

    /// Format as a fully-qualified image reference string.
    ///
    /// For docker.io with the `library` namespace, emits the short form
    /// (e.g. `alpine:latest`). Otherwise emits `registry/namespace/name:tag`.
    #[must_use]
    pub fn to_canonical_string(&self) -> String {
        if self.registry == "docker.io" && self.namespace == "library" {
            format!("{}:{}", self.name, self.tag)
        } else if self.registry == "docker.io" {
            format!("{}/{}:{}", self.namespace, self.name, self.tag)
        } else {
            format!(
                "{}/{}/{}:{}",
                self.registry, self.namespace, self.name, self.tag
            )
        }
    }

    #[must_use]
    /// Returns the HTTP registry host used for API requests.
    pub fn registry_host(&self) -> &str {
        match self.registry.as_str() {
            "docker.io" => "registry-1.docker.io",
            other => other,
        }
    }

    #[must_use]
    /// Returns the namespace-qualified repository path.
    pub fn repository(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }

    /// Storage key for `ImageStore`. Backward compat: docker.io returns "namespace/name"
    /// (no registry prefix) to preserve existing caches. All others prefix with registry.
    #[must_use]
    pub fn cache_name(&self) -> String {
        if self.registry == "docker.io" {
            format!("{}/{}", self.namespace, self.name)
        } else {
            format!("{}/{}/{}", self.registry, self.namespace, self.name)
        }
    }

    #[must_use]
    /// Returns the local cache path for this image reference.
    pub fn cache_path(&self, images_dir: &Path) -> PathBuf {
        if self.registry == "docker.io" {
            // Backward compat: docker.io omits registry prefix to preserve existing caches.
            images_dir
                .join(&self.namespace)
                .join(&self.name)
                .join(&self.tag)
        } else {
            images_dir
                .join(&self.registry)
                .join(&self.namespace)
                .join(&self.name)
                .join(&self.tag)
        }
    }
}

impl std::fmt::Display for ImageRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_canonical_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parse_bare_name() {
        let r = ImageRef::parse("alpine").unwrap();
        assert_eq!(r.registry, "docker.io");
        assert_eq!(r.namespace, "library");
        assert_eq!(r.name, "alpine");
        assert_eq!(r.tag, "latest");
    }

    #[test]
    fn parse_name_with_tag() {
        let r = ImageRef::parse("ubuntu:22.04").unwrap();
        assert_eq!(r.registry, "docker.io");
        assert_eq!(r.namespace, "library");
        assert_eq!(r.name, "ubuntu");
        assert_eq!(r.tag, "22.04");
    }

    #[test]
    fn parse_org_image() {
        let r = ImageRef::parse("myorg/myimage").unwrap();
        assert_eq!(r.registry, "docker.io");
        assert_eq!(r.namespace, "myorg");
        assert_eq!(r.name, "myimage");
        assert_eq!(r.tag, "latest");
    }

    #[test]
    fn parse_org_image_with_tag() {
        let r = ImageRef::parse("myorg/myimage:v2").unwrap();
        assert_eq!(r.registry, "docker.io");
        assert_eq!(r.namespace, "myorg");
        assert_eq!(r.name, "myimage");
        assert_eq!(r.tag, "v2");
    }

    #[test]
    fn parse_ghcr_full() {
        let r = ImageRef::parse("ghcr.io/org/minibox-rust-ci:stable").unwrap();
        assert_eq!(r.registry, "ghcr.io");
        assert_eq!(r.namespace, "org");
        assert_eq!(r.name, "minibox-rust-ci");
        assert_eq!(r.tag, "stable");
    }

    #[test]
    fn parse_empty_fails() {
        assert_eq!(ImageRef::parse(""), Err(ImageRefError::Empty));
    }

    #[test]
    fn parse_ghcr_without_namespace_fails() {
        assert!(ImageRef::parse("ghcr.io/image:tag").is_err());
    }

    #[test]
    fn registry_host_docker() {
        let r = ImageRef::parse("alpine").unwrap();
        assert_eq!(r.registry_host(), "registry-1.docker.io");
    }

    #[test]
    fn registry_host_ghcr() {
        let r = ImageRef::parse("ghcr.io/org/image:latest").unwrap();
        assert_eq!(r.registry_host(), "ghcr.io");
    }

    #[test]
    fn repository_docker() {
        let r = ImageRef::parse("alpine").unwrap();
        assert_eq!(r.repository(), "library/alpine");
    }

    #[test]
    fn repository_ghcr() {
        let r = ImageRef::parse("ghcr.io/org/minibox-rust-ci:stable").unwrap();
        assert_eq!(r.repository(), "org/minibox-rust-ci");
    }

    #[test]
    fn cache_name_docker_no_prefix() {
        let r = ImageRef::parse("alpine").unwrap();
        assert_eq!(r.cache_name(), "library/alpine");
    }

    #[test]
    fn cache_name_ghcr_with_prefix() {
        let r = ImageRef::parse("ghcr.io/org/image:stable").unwrap();
        assert_eq!(r.cache_name(), "ghcr.io/org/image");
    }

    #[test]
    fn cache_path_docker() {
        let r = ImageRef::parse("alpine").unwrap();
        let p = r.cache_path(Path::new("/data/images"));
        assert_eq!(
            p,
            std::path::PathBuf::from("/data/images/library/alpine/latest")
        );
    }

    #[test]
    fn cache_path_ghcr() {
        let r = ImageRef::parse("ghcr.io/org/image:stable").unwrap();
        let p = r.cache_path(Path::new("/data/images"));
        assert_eq!(
            p,
            std::path::PathBuf::from("/data/images/ghcr.io/org/image/stable")
        );
    }
}
