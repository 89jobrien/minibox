//! OCI image manifest and manifest list types.
//!
//! We support both single-arch manifests (`application/vnd.oci.image.manifest.v1+json`)
//! and multi-arch manifest lists (`application/vnd.oci.image.index.v1+json`
//! `application/vnd.docker.distribution.manifest.list.v2+json`).

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Media type constants
// ---------------------------------------------------------------------------

pub const MEDIA_TYPE_OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
pub const MEDIA_TYPE_OCI_INDEX: &str = "application/vnd.oci.image.index.v1+json";
pub const MEDIA_TYPE_DOCKER_MANIFEST: &str = "application/vnd.docker.distribution.manifest.v2+json";
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

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // ManifestResponse::parse — single-arch manifests
    // -------------------------------------------------------------------------

    #[test]
    fn parse_oci_single_manifest() {
        let body = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {
                "mediaType": "application/vnd.oci.image.config.v1+json",
                "size": 1234,
                "digest": "sha256:abc123"
            },
            "layers": [
                {
                    "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                    "size": 5678,
                    "digest": "sha256:def456"
                }
            ]
        });
        let json = serde_json::to_vec(&body).unwrap();

        let result = ManifestResponse::parse(&json, "application/vnd.oci.image.manifest.v1+json");
        assert!(result.is_ok());
        match result.unwrap() {
            ManifestResponse::Single(m) => {
                assert_eq!(m.schema_version, 2);
                assert_eq!(m.layers.len(), 1);
                assert_eq!(m.layers[0].digest, "sha256:def456");
            }
            ManifestResponse::List(_) => panic!("expected Single, got List"),
        }
    }

    #[test]
    fn parse_docker_v2_single_manifest() {
        let body = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
            "config": {
                "mediaType": "application/vnd.docker.container.image.v1+json",
                "size": 100,
                "digest": "sha256:config123"
            },
            "layers": []
        });
        let json = serde_json::to_vec(&body).unwrap();

        let result = ManifestResponse::parse(
            &json,
            "application/vnd.docker.distribution.manifest.v2+json",
        );
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), ManifestResponse::Single(_)));
    }

    // -------------------------------------------------------------------------
    // ManifestResponse::parse — manifest lists
    // -------------------------------------------------------------------------

    #[test]
    fn parse_docker_manifest_list() {
        let body = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.docker.distribution.manifest.list.v2+json",
            "manifests": [
                {
                    "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
                    "size": 528,
                    "digest": "sha256:linux_amd64",
                    "platform": { "architecture": "amd64", "os": "linux" }
                },
                {
                    "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
                    "size": 528,
                    "digest": "sha256:linux_arm64",
                    "platform": { "architecture": "arm64", "os": "linux" }
                }
            ]
        });
        let json = serde_json::to_vec(&body).unwrap();

        let result = ManifestResponse::parse(
            &json,
            "application/vnd.docker.distribution.manifest.list.v2+json",
        );
        assert!(result.is_ok());
        match result.unwrap() {
            ManifestResponse::List(list) => {
                assert_eq!(list.manifests.len(), 2);
            }
            ManifestResponse::Single(_) => panic!("expected List, got Single"),
        }
    }

    #[test]
    fn parse_oci_image_index() {
        let body = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": [
                {
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "size": 1000,
                    "digest": "sha256:arm_manifest",
                    "platform": { "architecture": "arm64", "os": "linux", "variant": "v8" }
                }
            ]
        });
        let json = serde_json::to_vec(&body).unwrap();

        let result =
            ManifestResponse::parse(&json, "application/vnd.oci.image.index.v1+json");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), ManifestResponse::List(_)));
    }

    // -------------------------------------------------------------------------
    // ManifestList::find_linux_amd64
    // -------------------------------------------------------------------------

    #[test]
    fn find_linux_amd64_returns_correct_descriptor() {
        let list = ManifestList {
            schema_version: 2,
            media_type: MEDIA_TYPE_DOCKER_MANIFEST_LIST.to_string(),
            manifests: vec![
                Descriptor {
                    media_type: MEDIA_TYPE_DOCKER_MANIFEST.to_string(),
                    size: 100,
                    digest: "sha256:arm_digest".to_string(),
                    platform: Some(Platform {
                        architecture: "arm64".to_string(),
                        os: "linux".to_string(),
                        variant: Some("v8".to_string()),
                    }),
                },
                Descriptor {
                    media_type: MEDIA_TYPE_DOCKER_MANIFEST.to_string(),
                    size: 200,
                    digest: "sha256:amd64_digest".to_string(),
                    platform: Some(Platform {
                        architecture: "amd64".to_string(),
                        os: "linux".to_string(),
                        variant: None,
                    }),
                },
            ],
        };

        let found = list.find_linux_amd64();
        assert!(found.is_some());
        assert_eq!(found.unwrap().digest, "sha256:amd64_digest");
    }

    #[test]
    fn find_linux_amd64_returns_none_when_missing() {
        let list = ManifestList {
            schema_version: 2,
            media_type: MEDIA_TYPE_OCI_INDEX.to_string(),
            manifests: vec![Descriptor {
                media_type: MEDIA_TYPE_OCI_MANIFEST.to_string(),
                size: 100,
                digest: "sha256:windows_digest".to_string(),
                platform: Some(Platform {
                    architecture: "amd64".to_string(),
                    os: "windows".to_string(),
                    variant: None,
                }),
            }],
        };

        assert!(list.find_linux_amd64().is_none());
    }

    #[test]
    fn find_linux_amd64_returns_none_for_empty_list() {
        let list = ManifestList {
            schema_version: 2,
            media_type: MEDIA_TYPE_OCI_INDEX.to_string(),
            manifests: vec![],
        };

        assert!(list.find_linux_amd64().is_none());
    }

    #[test]
    fn find_linux_amd64_skips_entries_without_platform() {
        let list = ManifestList {
            schema_version: 2,
            media_type: MEDIA_TYPE_DOCKER_MANIFEST_LIST.to_string(),
            manifests: vec![Descriptor {
                media_type: MEDIA_TYPE_DOCKER_MANIFEST.to_string(),
                size: 100,
                digest: "sha256:no_platform".to_string(),
                platform: None,
            }],
        };

        assert!(list.find_linux_amd64().is_none());
    }

    // -------------------------------------------------------------------------
    // parse — invalid JSON
    // -------------------------------------------------------------------------

    #[test]
    fn parse_returns_error_for_invalid_json() {
        let result = ManifestResponse::parse(b"not json", "application/vnd.oci.image.manifest.v1+json");
        assert!(result.is_err());
    }
}
