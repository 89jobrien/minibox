//! Conformance tests for minibox-macros — normalize/denormalize roundtrips,
//! macro expansion contracts.

use minibox_macros::{denormalize_digest, normalize, normalize_digest, normalize_name};

#[test]
fn conformance_normalize_name_replaces_slash() {
    assert_eq!(normalize_name!("library/alpine"), "library_alpine");
}

#[test]
fn conformance_normalize_name_no_slash_unchanged() {
    assert_eq!(normalize_name!("alpine"), "alpine");
}

#[test]
fn conformance_normalize_name_multiple_slashes() {
    assert_eq!(normalize_name!("ghcr.io/org/image"), "ghcr.io_org_image");
}

#[test]
fn conformance_normalize_digest_replaces_colon() {
    assert_eq!(normalize_digest!("sha256:abc123"), "sha256_abc123");
}

#[test]
fn conformance_normalize_digest_no_colon_unchanged() {
    assert_eq!(normalize_digest!("abc123"), "abc123");
}

#[test]
fn conformance_normalize_both_slash_and_colon() {
    assert_eq!(
        normalize!("ghcr.io/org/image:stable"),
        "ghcr.io_org_image_stable"
    );
}

#[test]
fn conformance_normalize_empty_string() {
    assert_eq!(normalize!(""), "");
}

#[test]
fn conformance_denormalize_digest_restores_colon() {
    assert_eq!(denormalize_digest!("sha256_abc123"), "sha256:abc123");
}

#[test]
fn conformance_normalize_denormalize_digest_roundtrip() {
    let original = "sha256:deadbeef";
    let normalized = normalize_digest!(original);
    let restored = denormalize_digest!(&normalized);
    assert_eq!(restored, original);
}

#[test]
fn conformance_normalize_name_with_port() {
    assert_eq!(
        normalize_name!("localhost:5000/myimage"),
        "localhost:5000_myimage"
    );
}
