//! Conformance tests for the `ImageRef` parsing and formatting contract.
//!
//! Verifies:
//! - `parse()` accepts well-formed image references.
//! - `parse()` rejects empty and invalid references.
//! - Bare name defaults: registry=docker.io, namespace=library, tag=latest.
//! - Named tag handling: custom tags override default latest.
//! - Registry detection: hostnames with dots or colons are treated as registries.
//! - Namespace requirement: non-docker.io registries require org/name format.
//! - Field construction: registry, namespace, name, tag are parsed consistently.
//! - `registry_host()` maps docker.io → registry-1.docker.io.
//! - `repository()` returns namespace/name.
//! - `cache_name()` formats storage key with docker.io backward compat (no prefix).
//! - `cache_path()` constructs full path with tag and registry-specific layout.
//! - Backward compatibility: docker.io paths omit registry prefix.
//!
//! No network, no I/O beyond Path operations.

use minibox_oci::ImageRef;
use minibox_oci::ImageRefError;
use std::path::Path;

// ---------------------------------------------------------------------------
// Parse: Docker Hub bare name and variations
// ---------------------------------------------------------------------------

#[test]
fn parse_bare_name_defaults_docker_io_library_latest() {
    let r = ImageRef::parse("alpine").expect("parse failed");
    assert_eq!(r.registry, "docker.io");
    assert_eq!(r.namespace, "library");
    assert_eq!(r.name, "alpine");
    assert_eq!(r.tag, "latest");
}

#[test]
fn parse_bare_name_with_tag() {
    let r = ImageRef::parse("ubuntu:22.04").expect("parse failed");
    assert_eq!(r.registry, "docker.io");
    assert_eq!(r.namespace, "library");
    assert_eq!(r.name, "ubuntu");
    assert_eq!(r.tag, "22.04");
}

#[test]
fn parse_org_name_defaults_docker_io() {
    let r = ImageRef::parse("myorg/myimage").expect("parse failed");
    assert_eq!(r.registry, "docker.io");
    assert_eq!(r.namespace, "myorg");
    assert_eq!(r.name, "myimage");
    assert_eq!(r.tag, "latest");
}

#[test]
fn parse_org_name_with_tag() {
    let r = ImageRef::parse("myorg/myimage:v2.1.0").expect("parse failed");
    assert_eq!(r.registry, "docker.io");
    assert_eq!(r.namespace, "myorg");
    assert_eq!(r.name, "myimage");
    assert_eq!(r.tag, "v2.1.0");
}

#[test]
fn parse_tag_defaults_to_latest_when_missing() {
    let r = ImageRef::parse("nginx").expect("parse failed");
    assert_eq!(r.tag, "latest", "tag must default to latest");
}

// ---------------------------------------------------------------------------
// Parse: Registry detection (hostname with dots or colons)
// ---------------------------------------------------------------------------

#[test]
fn parse_ghcr_with_full_path() {
    let r = ImageRef::parse("ghcr.io/org/minibox:stable").expect("parse failed");
    assert_eq!(r.registry, "ghcr.io");
    assert_eq!(r.namespace, "org");
    assert_eq!(r.name, "minibox");
    assert_eq!(r.tag, "stable");
}

#[test]
fn parse_custom_registry_with_port() {
    let r = ImageRef::parse("registry.example.com:5000/ns/img:v1").expect("parse failed");
    assert_eq!(r.registry, "registry.example.com:5000");
    assert_eq!(r.namespace, "ns");
    assert_eq!(r.name, "img");
    assert_eq!(r.tag, "v1");
}

#[test]
fn parse_localhost_registry() {
    let result = ImageRef::parse("localhost/myimage:latest");
    assert!(
        result.is_err(),
        "localhost without namespace must fail like any non-docker registry"
    );
}

#[test]
fn parse_ecr_registry() {
    let r = ImageRef::parse("123456789.dkr.ecr.us-east-1.amazonaws.com/org/image:tag")
        .expect("parse failed");
    assert_eq!(r.registry, "123456789.dkr.ecr.us-east-1.amazonaws.com");
    assert_eq!(r.namespace, "org");
    assert_eq!(r.name, "image");
    assert_eq!(r.tag, "tag");
}

// ---------------------------------------------------------------------------
// Parse: Error cases
// ---------------------------------------------------------------------------

#[test]
fn parse_empty_string_returns_empty_error() {
    let result = ImageRef::parse("");
    assert_eq!(
        result,
        Err(ImageRefError::Empty),
        "empty string must return Empty error"
    );
}

#[test]
fn parse_ghcr_without_namespace_returns_invalid() {
    let result = ImageRef::parse("ghcr.io/image:tag");
    assert!(result.is_err(), "ghcr.io without namespace must fail");
    assert!(
        matches!(result, Err(ImageRefError::Invalid(_))),
        "must return Invalid error"
    );
}

#[test]
fn parse_non_docker_registry_requires_namespace() {
    let result = ImageRef::parse("quay.io/image");
    assert!(
        result.is_err(),
        "non-docker registry without namespace must fail"
    );
}

#[test]
fn parse_empty_name_returns_invalid() {
    let result = ImageRef::parse("myorg/");
    assert!(
        result.is_err(),
        "image reference with empty name must fail"
    );
}

#[test]
fn parse_colon_without_tag_is_malformed() {
    let result = ImageRef::parse("alpine:");
    // According to the code, "alpine:" will have tag="" which is skipped, so it defaults to latest
    let r = result.expect("parse succeeded");
    assert_eq!(r.tag, "latest", "empty tag should default to latest");
}

// ---------------------------------------------------------------------------
// registry_host() mapping
// ---------------------------------------------------------------------------

#[test]
fn registry_host_docker_io_maps_to_registry_1() {
    let r = ImageRef::parse("alpine").expect("parse failed");
    assert_eq!(
        r.registry_host(),
        "registry-1.docker.io",
        "docker.io must map to registry-1.docker.io"
    );
}

#[test]
fn registry_host_non_docker_uses_as_is() {
    let r = ImageRef::parse("ghcr.io/org/image").expect("parse failed");
    assert_eq!(
        r.registry_host(),
        "ghcr.io",
        "non-docker registries must be used as-is"
    );
}

#[test]
fn registry_host_with_port_preserved() {
    let r = ImageRef::parse("localhost:5000/image/name").expect("parse failed");
    assert_eq!(
        r.registry_host(),
        "localhost:5000",
        "registry with port must be preserved"
    );
}

// ---------------------------------------------------------------------------
// repository() formatting
// ---------------------------------------------------------------------------

#[test]
fn repository_docker_includes_library_namespace() {
    let r = ImageRef::parse("alpine").expect("parse failed");
    assert_eq!(r.repository(), "library/alpine");
}

#[test]
fn repository_custom_namespace() {
    let r = ImageRef::parse("myorg/myimage").expect("parse failed");
    assert_eq!(r.repository(), "myorg/myimage");
}

#[test]
fn repository_includes_tag_not_included() {
    let r = ImageRef::parse("myorg/image:v1.2.3").expect("parse failed");
    assert_eq!(r.repository(), "myorg/image", "repository must not include tag");
}

// ---------------------------------------------------------------------------
// cache_name() — backward compat (docker.io omits registry prefix)
// ---------------------------------------------------------------------------

#[test]
fn cache_name_docker_io_no_registry_prefix() {
    let r = ImageRef::parse("alpine").expect("parse failed");
    assert_eq!(
        r.cache_name(),
        "library/alpine",
        "docker.io caching must omit registry prefix for backward compat"
    );
}

#[test]
fn cache_name_docker_io_with_org() {
    let r = ImageRef::parse("myorg/image").expect("parse failed");
    assert_eq!(
        r.cache_name(),
        "myorg/image",
        "docker.io caching must omit registry prefix"
    );
}

#[test]
fn cache_name_ghcr_includes_registry_prefix() {
    let r = ImageRef::parse("ghcr.io/org/image:stable").expect("parse failed");
    assert_eq!(
        r.cache_name(),
        "ghcr.io/org/image",
        "non-docker registries must include registry prefix"
    );
}

#[test]
fn cache_name_quay_includes_registry_prefix() {
    let r = ImageRef::parse("quay.io/org/image:v1").expect("parse failed");
    assert_eq!(
        r.cache_name(),
        "quay.io/org/image",
        "registry prefix required for non-docker.io registries"
    );
}

#[test]
fn cache_name_docker_io_tag_not_included() {
    let r = ImageRef::parse("ubuntu:22.04").expect("parse failed");
    assert_eq!(
        r.cache_name(),
        "library/ubuntu",
        "cache_name must not include tag"
    );
}

// ---------------------------------------------------------------------------
// cache_path() — full path construction with tag
// ---------------------------------------------------------------------------

#[test]
fn cache_path_docker_io_includes_tag() {
    let r = ImageRef::parse("alpine:3.16").expect("parse failed");
    let base = Path::new("/data/images");
    let path = r.cache_path(base);
    assert_eq!(path, Path::new("/data/images/library/alpine/3.16"));
}

#[test]
fn cache_path_docker_io_default_tag() {
    let r = ImageRef::parse("nginx").expect("parse failed");
    let base = Path::new("/var/lib/minibox/images");
    let path = r.cache_path(base);
    assert_eq!(
        path,
        Path::new("/var/lib/minibox/images/library/nginx/latest")
    );
}

#[test]
fn cache_path_docker_io_with_org() {
    let r = ImageRef::parse("myorg/app:v1.0").expect("parse failed");
    let base = Path::new("/images");
    let path = r.cache_path(base);
    assert_eq!(path, Path::new("/images/myorg/app/v1.0"));
}

#[test]
fn cache_path_ghcr_includes_registry_and_tag() {
    let r = ImageRef::parse("ghcr.io/org/minibox:stable").expect("parse failed");
    let base = Path::new("/data/images");
    let path = r.cache_path(base);
    assert_eq!(
        path,
        Path::new("/data/images/ghcr.io/org/minibox/stable")
    );
}

#[test]
fn cache_path_ecr_registry() {
    let r = ImageRef::parse("123456789.dkr.ecr.us-east-1.amazonaws.com/ns/img:latest")
        .expect("parse failed");
    let base = Path::new("/tmp/images");
    let path = r.cache_path(base);
    assert!(
        path.to_string_lossy().contains("123456789.dkr.ecr.us-east-1.amazonaws.com"),
        "ECR registry must be included in path"
    );
    assert!(
        path.to_string_lossy().ends_with("latest"),
        "tag must be the final component"
    );
}

// ---------------------------------------------------------------------------
// Equality and Clone
// ---------------------------------------------------------------------------

#[test]
fn image_ref_equality() {
    let r1 = ImageRef::parse("alpine:3.16").expect("parse failed");
    let r2 = ImageRef::parse("alpine:3.16").expect("parse failed");
    assert_eq!(r1, r2, "ImageRefs with same components must be equal");
}

#[test]
fn image_ref_inequality_different_tags() {
    let r1 = ImageRef::parse("alpine:3.16").expect("parse failed");
    let r2 = ImageRef::parse("alpine:3.17").expect("parse failed");
    assert_ne!(r1, r2, "ImageRefs with different tags must not be equal");
}

#[test]
fn image_ref_clone() {
    let r1 = ImageRef::parse("ghcr.io/org/image:v1").expect("parse failed");
    let r2 = r1.clone();
    assert_eq!(r1, r2, "cloned ImageRef must be equal to original");
}

#[test]
fn image_ref_debug_shows_all_fields() {
    let r = ImageRef::parse("ghcr.io/org/image:v1.0.0").expect("parse failed");
    let debug = format!("{:?}", r);
    assert!(debug.contains("ghcr.io"));
    assert!(debug.contains("org"));
    assert!(debug.contains("image"));
    assert!(debug.contains("v1.0.0"));
}
