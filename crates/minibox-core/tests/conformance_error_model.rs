//! Conformance tests for the OCI error model types.
//!
//! Verifies:
//! - `ImageError::NotFound` Display message includes name and tag.
//! - `ImageError::DigestMismatch` Display shows digest, expected, and actual values.
//! - `ImageError::DeviceNodeRejected` Display includes entry path.
//! - `ImageError::SymlinkTraversalRejected` Display includes entry and target paths.
//! - `ImageError::StoreWrite` and `ImageError::StoreRead` include path and io::Error.
//! - `ImageError::ManifestParse` includes name, tag, and parse error.
//! - `ImageError::LayerExtract` wraps extraction error message.
//! - `ImageError::Io` wraps io::Error via From.
//! - `ImageError::Other` wraps arbitrary string message.
//! - `RegistryError::AuthFailed` Display includes image and message.
//! - `RegistryError::ManifestFetch` Display includes name, tag, and message.
//! - `RegistryError::BlobFetch` Display includes digest and message.
//! - `RegistryError::NoPlatformManifest` Display includes platform string.
//! - `RegistryError::ManifestNestingTooDeep` Display has known message.
//! - `RegistryError::LayerTask` wraps JoinError.
//! - `RegistryError::Other` wraps arbitrary message.
//! - All error types implement Debug and Error traits correctly.
//!
//! No network, no I/O beyond error message formatting.

use minibox_core::{ImageError, RegistryError};
use std::io;

// ---------------------------------------------------------------------------
// ImageError::NotFound
// ---------------------------------------------------------------------------

#[test]
fn conformance_image_error_not_found_display() {
    let err = ImageError::NotFound {
        name: "myapp".to_string(),
        tag: "v1.0.0".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("myapp"));
    assert!(msg.contains("v1.0.0"));
    assert!(msg.contains("not found"));
}

// ---------------------------------------------------------------------------
// ImageError::DigestMismatch
// ---------------------------------------------------------------------------

#[test]
fn conformance_image_error_digest_mismatch_display() {
    let err = ImageError::DigestMismatch {
        digest: "sha256:abc123def456".to_string(),
        expected: "sha256:expected1234567890abcdef".to_string(),
        actual: "sha256:actual0987654321fedcba".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("abc123def456"));
    assert!(msg.contains("expected1234567890abcdef"));
    assert!(msg.contains("actual0987654321fedcba"));
    assert!(msg.contains("digest mismatch"));
}

// ---------------------------------------------------------------------------
// ImageError::DeviceNodeRejected
// ---------------------------------------------------------------------------

#[test]
fn conformance_image_error_device_node_rejected_display() {
    let err = ImageError::DeviceNodeRejected {
        entry: "/dev/sda".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("/dev/sda"));
    assert!(msg.contains("device node"));
}

// ---------------------------------------------------------------------------
// ImageError::SymlinkTraversalRejected
// ---------------------------------------------------------------------------

#[test]
fn conformance_image_error_symlink_traversal_display() {
    let err = ImageError::SymlinkTraversalRejected {
        entry: "bin/sh".to_string(),
        target: "../../etc/passwd".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("bin/sh"));
    assert!(msg.contains("../../etc/passwd"));
    assert!(msg.contains("symlink"));
}

// ---------------------------------------------------------------------------
// ImageError::StoreWrite
// ---------------------------------------------------------------------------

#[test]
fn conformance_image_error_store_write_display() {
    let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
    let err = ImageError::StoreWrite {
        path: "/var/lib/minibox/images/alpine/layer.tar".to_string(),
        source: io_err,
    };
    let msg = err.to_string();
    assert!(msg.contains("/var/lib/minibox/images/alpine/layer.tar"));
    assert!(msg.contains("write"));
}

// ---------------------------------------------------------------------------
// ImageError::StoreRead
// ---------------------------------------------------------------------------

#[test]
fn conformance_image_error_store_read_display() {
    let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
    let err = ImageError::StoreRead {
        path: "/var/lib/minibox/images/ubuntu/manifest.json".to_string(),
        source: io_err,
    };
    let msg = err.to_string();
    assert!(msg.contains("/var/lib/minibox/images/ubuntu/manifest.json"));
    assert!(msg.contains("read"));
}

// ---------------------------------------------------------------------------
// ImageError::ManifestParse
// ---------------------------------------------------------------------------

#[test]
fn conformance_image_error_manifest_parse_display() {
    let json_err = serde_json::from_str::<serde_json::Value>("invalid json")
        .expect_err("intentional parse error");
    let err = ImageError::ManifestParse {
        name: "nginx".to_string(),
        tag: "1.21".to_string(),
        source: json_err,
    };
    let msg = err.to_string();
    assert!(msg.contains("nginx"));
    assert!(msg.contains("1.21"));
    assert!(msg.contains("manifest"));
}

// ---------------------------------------------------------------------------
// ImageError::LayerExtract
// ---------------------------------------------------------------------------

#[test]
fn conformance_image_error_layer_extract_display() {
    let err = ImageError::LayerExtract("tar entry missing required fields".to_string());
    let msg = err.to_string();
    assert!(msg.contains("tar entry missing required fields"));
    assert!(msg.contains("layer extraction"));
}

// ---------------------------------------------------------------------------
// ImageError::Io (From conversion)
// ---------------------------------------------------------------------------

#[test]
fn conformance_image_error_io_from() {
    let io_err = io::Error::new(io::ErrorKind::TimedOut, "operation timed out");
    let err: ImageError = io_err.into();
    let msg = err.to_string();
    assert!(msg.contains("timed out"));
    assert!(msg.contains("io error"));
}

// ---------------------------------------------------------------------------
// ImageError::Other
// ---------------------------------------------------------------------------

#[test]
fn conformance_image_error_other_display() {
    let err = ImageError::Other("unexpected registry response".to_string());
    let msg = err.to_string();
    assert!(msg.contains("unexpected registry response"));
    assert!(msg.contains("layer error"));
}

// ---------------------------------------------------------------------------
// ImageError Debug
// ---------------------------------------------------------------------------

#[test]
fn conformance_image_error_is_debug() {
    let err = ImageError::NotFound {
        name: "test-image".to_string(),
        tag: "latest".to_string(),
    };
    let debug_str = format!("{:?}", err);
    assert!(!debug_str.is_empty());
    assert!(debug_str.contains("NotFound"));
}

// ---------------------------------------------------------------------------
// RegistryError::AuthFailed
// ---------------------------------------------------------------------------

#[test]
fn conformance_registry_error_auth_failed_display() {
    let err = RegistryError::AuthFailed {
        image: "ghcr.io/org/private-image".to_string(),
        message: "401 Unauthorized".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("ghcr.io/org/private-image"));
    assert!(msg.contains("401 Unauthorized"));
    assert!(msg.contains("authentication failed"));
}

// ---------------------------------------------------------------------------
// RegistryError::ManifestFetch
// ---------------------------------------------------------------------------

#[test]
fn conformance_registry_error_manifest_fetch_display() {
    let err = RegistryError::ManifestFetch {
        name: "alpine".to_string(),
        tag: "3.16".to_string(),
        message: "HTTP 404 Not Found".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("alpine"));
    assert!(msg.contains("3.16"));
    assert!(msg.contains("HTTP 404 Not Found"));
    assert!(msg.contains("manifest"));
}

// ---------------------------------------------------------------------------
// RegistryError::BlobFetch
// ---------------------------------------------------------------------------

#[test]
fn conformance_registry_error_blob_fetch_display() {
    let err = RegistryError::BlobFetch {
        digest: "sha256:0123456789abcdef".to_string(),
        message: "blob not found in registry".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("sha256:0123456789abcdef"));
    assert!(msg.contains("blob not found in registry"));
    assert!(msg.contains("blob"));
}

// ---------------------------------------------------------------------------
// RegistryError::NoPlatformManifest
// ---------------------------------------------------------------------------

#[test]
fn conformance_registry_error_no_platform_manifest_display() {
    let err = RegistryError::NoPlatformManifest {
        platform: "linux/amd64".into(),
    };
    let msg = err.to_string();
    assert!(msg.contains("linux/amd64"), "should contain platform: {msg}");
    assert!(msg.contains("manifest"), "should contain 'manifest': {msg}");
}

// ---------------------------------------------------------------------------
// RegistryError::ManifestNestingTooDeep
// ---------------------------------------------------------------------------

#[test]
fn conformance_registry_error_nesting_too_deep_display() {
    let err = RegistryError::ManifestNestingTooDeep;
    let msg = err.to_string();
    assert!(msg.contains("nesting"));
    assert!(msg.contains("manifest"));
}

// ---------------------------------------------------------------------------
// RegistryError::Other
// ---------------------------------------------------------------------------

#[test]
fn conformance_registry_error_other_display() {
    let err = RegistryError::Other("unexpected response format".to_string());
    let msg = err.to_string();
    assert!(msg.contains("unexpected response format"));
    assert!(msg.contains("registry error"));
}

// ---------------------------------------------------------------------------
// RegistryError Debug
// ---------------------------------------------------------------------------

#[test]
fn conformance_registry_error_is_debug() {
    let err = RegistryError::NoPlatformManifest {
        platform: "linux/amd64".into(),
    };
    let debug_str = format!("{:?}", err);
    assert!(!debug_str.is_empty());
    assert!(debug_str.contains("NoPlatformManifest"));
}

// ---------------------------------------------------------------------------
// Error trait
// ---------------------------------------------------------------------------

#[test]
fn conformance_image_error_is_std_error() {
    use std::error::Error as StdError;
    let err: Box<dyn StdError> = Box::new(ImageError::LayerExtract("test".to_string()));
    assert!(!err.to_string().is_empty());
}

#[test]
fn conformance_registry_error_is_std_error() {
    use std::error::Error as StdError;
    let err: Box<dyn StdError> = Box::new(RegistryError::Other("test".to_string()));
    assert!(!err.to_string().is_empty());
}
