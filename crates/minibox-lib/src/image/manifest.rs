//! OCI image manifest and manifest list types.
//!
//! We support both single-arch manifests
//! (`application/vnd.oci.image.manifest.v1+json`) and multi-arch manifest
//! lists (`application/vnd.oci.image.index.v1+json` /
//! `application/vnd.docker.distribution.manifest.list.v2+json`).

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Media type constants
// ---------------------------------------------------------------------------

pub const MEDIA_TYPE_OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
pub const MEDIA_TYPE_OCI_INDEX: &str = "application/vnd.oci.image.index.v1+json";
pub const MEDIA_TYPE_DOCKER_MANIFEST: &str =
    "application/vnd.docker.distribution.manifest.v2+json";
pub const MEDIA_TYPE_DOCKER_MANIFEST_LIST: &str =
    "application/vnd.docker.distribution.manifest.list.v2+json";

// ---------------------------------------------------------------------------
// Descriptor
// ---------------------------------------------------------------------------

/// A content-addressed blob descriptor used inside manifests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Descriptor {
    /// MIME type of the referenced blob.
    #[serde(rename = "mediaType")]
    pub media_type: String,
    /// Size of the blob in bytes.
    pub size: u64,
    /// `sha256:...` digest of the blob.
    pub digest: String,
    /// Optional platform annotation (present in manifest list entries).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
}

// ---------------------------------------------------------------------------
// Platform
// ---------------------------------------------------------------------------

/// Platform annotation on a manifest list entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Platform {
    pub architecture: String,
    pub os: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

// ---------------------------------------------------------------------------
// OCI single-arch manifest
// ---------------------------------------------------------------------------

/// An OCI image manifest (`application/vnd.oci.image.manifest.v1+json`).
///
/// Also parses Docker v2 schema 2 manifests which share the same shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciManifest {
    /// Must be `2` for OCI / Docker v2 manifests.
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    /// Media type identifying the manifest format.
    #[serde(rename = "mediaType", default)]
    pub media_type: String,
    /// Descriptor for the image config blob.
    pub config: Descriptor,
    /// Ordered list of layer descriptors (bottom-to-top).
    pub layers: Vec<Descriptor>,
}

// ---------------------------------------------------------------------------
// Manifest list / image index
// ---------------------------------------------------------------------------

/// An OCI image index or Docker manifest list.
///
/// Used for multi-arch images; each entry points to a single-arch manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestList {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(rename = "mediaType", default)]
    pub media_type: String,
    /// List of platform-specific manifest descriptors.
    pub manifests: Vec<Descriptor>,
}

impl ManifestList {
    /// Find the descriptor for the `linux/amd64` platform.
    pub fn find_linux_amd64(&self) -> Option<&Descriptor> {
        self.manifests.iter().find(|d| {
            if let Some(p) = &d.platform {
                p.os == "linux" && p.architecture == "amd64"
            } else {
                false
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Unified response type
// ---------------------------------------------------------------------------

/// The result of fetching a manifest endpoint -- either a single-arch manifest
/// or a multi-arch manifest list.
#[derive(Debug, Clone)]
pub enum ManifestResponse {
    Single(OciManifest),
    List(ManifestList),
}

impl ManifestResponse {
    /// Deserialize from raw JSON bytes given a `Content-Type` / media type header.
    pub fn parse(body: &[u8], media_type: &str) -> anyhow::Result<Self> {
        if media_type.contains("manifest.list") || media_type.contains("image.index") {
            let list: ManifestList = serde_json::from_slice(body)?;
            Ok(ManifestResponse::List(list))
        } else {
            let manifest: OciManifest = serde_json::from_slice(body)?;
            Ok(ManifestResponse::Single(manifest))
        }
    }
}
