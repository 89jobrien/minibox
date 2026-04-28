//! OCI image manifest and manifest list types.
//!
//! We support both single-arch manifests (`application/vnd.oci.image.manifest.v1+json`)
//! and multi-arch manifest lists (`application/vnd.oci.image.index.v1+json`
//! `application/vnd.docker.distribution.manifest.list.v2+json`).

use anyhow::bail;
use serde::{Deserialize, Serialize};
use std::fmt;

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
///
/// Matches Docker Hub / OCI platform identifiers (e.g. `os = "linux"`,
/// `architecture = "amd64"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Platform {
    /// CPU architecture (e.g. `"amd64"`, `"arm64"`).
    pub architecture: String,
    /// Operating system (e.g. `"linux"`, `"windows"`).
    pub os: String,
    /// Architecture variant for ARM (e.g. `"v7"`, `"v8"`). `None` for amd64.
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
/// Used for multi-arch images; each entry in `manifests` points to a
/// single-arch [`OciManifest`] identified by digest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestList {
    /// Must be `2` for OCI / Docker v2 manifest lists.
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    /// Media type of this manifest list (e.g. [`MEDIA_TYPE_OCI_INDEX`] or
    /// [`MEDIA_TYPE_DOCKER_MANIFEST_LIST`]).
    #[serde(rename = "mediaType", default)]
    pub media_type: String,
    /// Platform-specific manifest descriptors (one per supported platform).
    pub manifests: Vec<Descriptor>,
}

impl ManifestList {
    /// Find the descriptor matching a specific [`TargetPlatform`].
    ///
    /// Matches on `os` and `architecture`. If the target has a `variant`, the
    /// descriptor must also match that variant. If the target has no variant,
    /// any variant (or none) on the descriptor is accepted.
    pub fn find_platform(&self, target: &TargetPlatform) -> Option<&Descriptor> {
        self.manifests.iter().find(|d| {
            if let Some(p) = &d.platform {
                if p.os != target.os || p.architecture != target.architecture {
                    return false;
                }
                // If the target specifies a variant, it must match.
                if let Some(tv) = &target.variant {
                    p.variant.as_ref() == Some(tv)
                } else {
                    true
                }
            } else {
                false
            }
        })
    }

    /// Find the descriptor for the `linux/amd64` platform.
    pub fn find_linux_amd64(&self) -> Option<&Descriptor> {
        self.find_platform(&TargetPlatform::linux_amd64())
    }
}

// ---------------------------------------------------------------------------
// Unified response type
// ---------------------------------------------------------------------------

/// The result of fetching a manifest endpoint — either a single-arch manifest
/// or a multi-arch manifest list.
///
/// Construct via [`ManifestResponse::parse`] which inspects the `Content-Type`
/// header to decide which variant to deserialize.
#[derive(Debug, Clone)]
pub enum ManifestResponse {
    /// A single-architecture image manifest.
    Single(OciManifest),
    /// A multi-architecture manifest list / OCI image index.
    List(ManifestList),
}

impl ManifestResponse {
    /// Deserialize a manifest response from raw JSON bytes.
    ///
    /// `media_type` is the value of the HTTP `Content-Type` header returned by
    /// the registry. The function dispatches on substrings:
    ///
    /// - Contains `"manifest.list"` → [`ManifestResponse::List`] (Docker manifest list)
    /// - Contains `"image.index"` → [`ManifestResponse::List`] (OCI image index)
    /// - Anything else → [`ManifestResponse::Single`] (single-arch manifest)
    ///
    /// # Errors
    ///
    /// Returns an error if `body` is not valid JSON for the inferred variant.
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

// ---------------------------------------------------------------------------
// TargetPlatform
// ---------------------------------------------------------------------------

/// A target platform for multi-arch image selection.
///
/// Combines OS, architecture, and optional variant (e.g. ARM v7/v8) to match
/// against manifest list entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetPlatform {
    /// Operating system (e.g. `"linux"`, `"windows"`).
    pub os: String,
    /// CPU architecture in OCI convention (e.g. `"amd64"`, `"arm64"`).
    pub architecture: String,
    /// Architecture variant (e.g. `"v8"` for ARM64). `None` for amd64.
    pub variant: Option<String>,
}

impl Default for TargetPlatform {
    /// Auto-detect the host platform, mapping Rust arch names to OCI conventions.
    fn default() -> Self {
        let os = std::env::consts::OS.to_string();
        let architecture = match std::env::consts::ARCH {
            "aarch64" => "arm64".to_string(),
            "x86_64" => "amd64".to_string(),
            other => other.to_string(),
        };
        Self {
            os,
            architecture,
            variant: None,
        }
    }
}

impl TargetPlatform {
    /// Parse a platform string in `"os/arch"` or `"os/arch/variant"` format.
    ///
    /// # Errors
    ///
    /// Returns an error if the string does not contain at least `os/arch`.
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        let parts: Vec<&str> = s.split('/').collect();
        match parts.len() {
            2 => Ok(Self {
                os: parts[0].to_string(),
                architecture: parts[1].to_string(),
                variant: None,
            }),
            3 => Ok(Self {
                os: parts[0].to_string(),
                architecture: parts[1].to_string(),
                variant: Some(parts[2].to_string()),
            }),
            _ => bail!("invalid platform string: expected os/arch or os/arch/variant, got {s:?}"),
        }
    }

    /// Convenience constructor for `linux/amd64`.
    pub fn linux_amd64() -> Self {
        Self {
            os: "linux".to_string(),
            architecture: "amd64".to_string(),
            variant: None,
        }
    }
}

impl fmt::Display for TargetPlatform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.os, self.architecture)?;
        if let Some(v) = &self.variant {
            write!(f, "/{v}")?;
        }
        Ok(())
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

        let result = ManifestResponse::parse(&json, "application/vnd.oci.image.index.v1+json");
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
        let result =
            ManifestResponse::parse(b"not json", "application/vnd.oci.image.manifest.v1+json");
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // TargetPlatform
    // -------------------------------------------------------------------------

    #[test]
    fn target_platform_default_matches_host() {
        let tp = TargetPlatform::default();
        assert!(!tp.os.is_empty(), "os should be non-empty");
        assert!(
            !tp.architecture.is_empty(),
            "architecture should be non-empty"
        );
        assert_eq!(tp.os, std::env::consts::OS);
    }

    #[test]
    fn target_platform_parse_two_part() {
        let tp = TargetPlatform::parse("linux/arm64").expect("should parse");
        assert_eq!(tp.os, "linux");
        assert_eq!(tp.architecture, "arm64");
        assert_eq!(tp.variant, None);
    }

    #[test]
    fn target_platform_parse_three_part() {
        let tp = TargetPlatform::parse("linux/arm64/v8").expect("should parse");
        assert_eq!(tp.os, "linux");
        assert_eq!(tp.architecture, "arm64");
        assert_eq!(tp.variant, Some("v8".to_string()));
    }

    #[test]
    fn target_platform_parse_invalid() {
        assert!(TargetPlatform::parse("invalid").is_err());
    }

    #[test]
    fn target_platform_display_without_variant() {
        let tp = TargetPlatform {
            os: "linux".to_string(),
            architecture: "amd64".to_string(),
            variant: None,
        };
        assert_eq!(tp.to_string(), "linux/amd64");
    }

    #[test]
    fn target_platform_display_with_variant() {
        let tp = TargetPlatform {
            os: "linux".to_string(),
            architecture: "arm64".to_string(),
            variant: Some("v8".to_string()),
        };
        assert_eq!(tp.to_string(), "linux/arm64/v8");
    }

    #[test]
    fn target_platform_linux_amd64() {
        let tp = TargetPlatform::linux_amd64();
        assert_eq!(tp.os, "linux");
        assert_eq!(tp.architecture, "amd64");
        assert_eq!(tp.variant, None);
    }

    // -------------------------------------------------------------------------
    // ManifestList::find_platform
    // -------------------------------------------------------------------------

    #[test]
    fn find_platform_matches_linux_amd64() {
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
        let target = TargetPlatform::linux_amd64();
        let found = list.find_platform(&target);
        assert!(found.is_some());
        assert_eq!(found.expect("should find").digest, "sha256:amd64_digest");
    }

    #[test]
    fn find_platform_matches_arm64_with_variant() {
        let list = ManifestList {
            schema_version: 2,
            media_type: MEDIA_TYPE_OCI_INDEX.to_string(),
            manifests: vec![Descriptor {
                media_type: MEDIA_TYPE_OCI_MANIFEST.to_string(),
                size: 100,
                digest: "sha256:arm_v8".to_string(),
                platform: Some(Platform {
                    architecture: "arm64".to_string(),
                    os: "linux".to_string(),
                    variant: Some("v8".to_string()),
                }),
            }],
        };
        let target = TargetPlatform::parse("linux/arm64/v8").expect("parse");
        let found = list.find_platform(&target);
        assert!(found.is_some());
        assert_eq!(found.expect("should find").digest, "sha256:arm_v8");
    }

    #[test]
    fn find_platform_returns_none_for_missing() {
        let list = ManifestList {
            schema_version: 2,
            media_type: MEDIA_TYPE_OCI_INDEX.to_string(),
            manifests: vec![Descriptor {
                media_type: MEDIA_TYPE_OCI_MANIFEST.to_string(),
                size: 100,
                digest: "sha256:win".to_string(),
                platform: Some(Platform {
                    architecture: "amd64".to_string(),
                    os: "windows".to_string(),
                    variant: None,
                }),
            }],
        };
        let target = TargetPlatform::linux_amd64();
        assert!(list.find_platform(&target).is_none());
    }

    #[test]
    fn find_platform_ignores_variant_when_target_has_none() {
        let list = ManifestList {
            schema_version: 2,
            media_type: MEDIA_TYPE_OCI_INDEX.to_string(),
            manifests: vec![Descriptor {
                media_type: MEDIA_TYPE_OCI_MANIFEST.to_string(),
                size: 100,
                digest: "sha256:arm_v8".to_string(),
                platform: Some(Platform {
                    architecture: "arm64".to_string(),
                    os: "linux".to_string(),
                    variant: Some("v8".to_string()),
                }),
            }],
        };
        // Target has no variant — should still match arm64/linux
        let target = TargetPlatform::parse("linux/arm64").expect("parse");
        let found = list.find_platform(&target);
        assert!(found.is_some());
    }

    #[test]
    fn find_linux_amd64_still_works() {
        let list = ManifestList {
            schema_version: 2,
            media_type: MEDIA_TYPE_DOCKER_MANIFEST_LIST.to_string(),
            manifests: vec![Descriptor {
                media_type: MEDIA_TYPE_DOCKER_MANIFEST.to_string(),
                size: 200,
                digest: "sha256:amd64_digest".to_string(),
                platform: Some(Platform {
                    architecture: "amd64".to_string(),
                    os: "linux".to_string(),
                    variant: None,
                }),
            }],
        };
        assert!(list.find_linux_amd64().is_some());
    }
}
