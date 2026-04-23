//! Property-based tests for image digest verification and manifest parsing.

use linuxbox::image::layer::verify_digest;
use linuxbox::image::manifest::{
    Descriptor, ManifestList, ManifestResponse, OciManifest, Platform,
};
use proptest::prelude::*;
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn make_descriptor(media_type: &str, digest: &str, size: u64) -> Descriptor {
    Descriptor {
        media_type: media_type.to_string(),
        digest: digest.to_string(),
        size,
        platform: None,
    }
}

fn make_descriptor_with_platform(
    media_type: &str,
    digest: &str,
    size: u64,
    arch: &str,
    os: &str,
) -> Descriptor {
    Descriptor {
        media_type: media_type.to_string(),
        digest: digest.to_string(),
        size,
        platform: Some(Platform {
            architecture: arch.to_string(),
            os: os.to_string(),
            variant: None,
        }),
    }
}

fn oci_manifest_json(schema_version: u32, config_digest: &str, layer_digests: &[&str]) -> Vec<u8> {
    let layers: Vec<serde_json::Value> = layer_digests
        .iter()
        .map(|d| {
            serde_json::json!({
                "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                "size": 1000u64,
                "digest": d
            })
        })
        .collect();

    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": schema_version,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "size": 512u64,
            "digest": config_digest
        },
        "layers": layers
    }))
    .unwrap()
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    // -----------------------------------------------------------------------
    // Digest verification
    // -----------------------------------------------------------------------

    /// verify_digest accepts a correctly computed SHA256 digest.
    #[test]
    fn verify_digest_accepts_correct_sha256(
        data in prop::collection::vec(any::<u8>(), 0..1024)
    ) {
        let digest = format!("sha256:{}", sha256_hex(&data));
        prop_assert!(
            verify_digest(&data, &digest).is_ok(),
            "expected Ok for correct digest of {} bytes",
            data.len()
        );
    }

    /// verify_digest rejects a digest string that doesn't match the actual hash.
    ///
    /// We generate a random hex string of the right length (64 chars) but
    /// XOR the first byte so it can never accidentally match.
    #[test]
    fn verify_digest_rejects_wrong_digest(
        data in prop::collection::vec(any::<u8>(), 1..512),
        wrong_suffix in "[0-9a-f]{64}"
    ) {
        // Ensure the wrong suffix actually differs from the real digest.
        let real_hex = sha256_hex(&data);
        // Flip the last nibble of wrong_suffix to guarantee mismatch.
        let mut wrong = wrong_suffix.clone();
        let last = wrong.pop().unwrap();
        let flipped = if last == 'f' { '0' } else { 'f' };
        if wrong == &real_hex[..63] {
            // The prefix matched — append the flipped char to guarantee divergence.
            wrong.push(flipped);
        } else {
            wrong.push(last);
        }

        let digest = format!("sha256:{wrong}");
        // If they accidentally match (extremely unlikely), skip; otherwise assert error.
        if wrong == real_hex {
            // Skip — proptest found a 1-in-2^256 collision; treat as vacuous pass.
            return Ok(());
        }
        prop_assert!(
            verify_digest(&data, &digest).is_err(),
            "expected Err for mismatched digest"
        );
    }

    /// verify_digest rejects strings without the `sha256:` prefix.
    #[test]
    fn verify_digest_rejects_missing_prefix(
        data in prop::collection::vec(any::<u8>(), 0..256),
        hex_str in "[0-9a-f]{64}"
    ) {
        // A bare hex string (no "sha256:" prefix) must always be rejected.
        prop_assert!(
            verify_digest(&data, &hex_str).is_err(),
            "expected Err for digest without sha256: prefix"
        );
    }

    /// verify_digest rejects strings with a `sha256:` prefix but wrong-length hex.
    #[test]
    fn verify_digest_rejects_wrong_length_hex(
        data in prop::collection::vec(any::<u8>(), 0..256),
        // Lengths 0..63 and 65..128 — deliberately avoiding the correct 64.
        bad_hex in prop_oneof![
            "[0-9a-f]{0,63}",
            "[0-9a-f]{65,128}"
        ]
    ) {
        let digest = format!("sha256:{bad_hex}");
        // A correct digest would be exactly 64 hex chars; wrong length must fail.
        // (It may or may not match depending on length — but length != 64 should
        //  not accidentally equal a well-formed digest, so we always expect an error.)
        let result = verify_digest(&data, &digest);
        // Only assert error when the bad_hex length is not 64 chars.
        if bad_hex.len() != 64 {
            prop_assert!(
                result.is_err(),
                "expected Err for wrong-length hex ({} chars)",
                bad_hex.len()
            );
        }
    }

    /// SHA256 is deterministic: same input always produces the same digest.
    #[test]
    fn sha256_is_deterministic(
        data in prop::collection::vec(any::<u8>(), 0..1024)
    ) {
        let digest_a = format!("sha256:{}", sha256_hex(&data));
        let digest_b = format!("sha256:{}", sha256_hex(&data));
        prop_assert_eq!(&digest_a, &digest_b);
        prop_assert!(verify_digest(&data, &digest_a).is_ok());
        prop_assert!(verify_digest(&data, &digest_b).is_ok());
    }

    // -----------------------------------------------------------------------
    // Manifest parsing — OCI single manifest
    // -----------------------------------------------------------------------

    /// A valid OCI manifest JSON roundtrips through ManifestResponse::parse.
    #[test]
    fn valid_oci_manifest_roundtrips(
        schema_version in 1u32..=3,
        config_tag in "[a-f0-9]{8}",
        layer_tags in prop::collection::vec("[a-f0-9]{8}", 0..4),
    ) {
        let config_digest = format!("sha256:{config_tag:0<64}");
        let layer_digests: Vec<String> = layer_tags
            .iter()
            .map(|t| format!("sha256:{t:0<64}"))
            .collect();
        let layer_count = layer_digests.len();
        let layer_digest_refs: Vec<&str> = layer_digests.iter().map(String::as_str).collect();

        let body = oci_manifest_json(schema_version, &config_digest, &layer_digest_refs);
        let result = ManifestResponse::parse(
            &body,
            "application/vnd.oci.image.manifest.v1+json",
        );

        prop_assert!(result.is_ok(), "parse failed: {:?}", result.err());
        match result.unwrap() {
            ManifestResponse::Single(m) => {
                prop_assert_eq!(m.schema_version, schema_version);
                prop_assert_eq!(m.config.digest, config_digest);
                prop_assert_eq!(m.layers.len(), layer_count);
                for (i, layer) in m.layers.iter().enumerate() {
                    prop_assert_eq!(&layer.digest, &layer_digests[i]);
                }
            }
            ManifestResponse::List(_) => {
                prop_assert!(false, "expected Single, got List");
            }
        }
    }

    // -----------------------------------------------------------------------
    // Manifest parsing — manifest list
    // -----------------------------------------------------------------------

    /// ManifestList::find_linux_amd64 always finds the correct entry when present.
    #[test]
    fn manifest_list_find_linux_amd64_finds_correct_entry(
        // Other platforms to include alongside linux/amd64
        other_platforms in prop::collection::vec(
            prop_oneof![
                Just(("arm64", "linux")),
                Just(("amd64", "windows")),
                Just(("arm64", "windows")),
                Just(("s390x", "linux")),
            ],
            0..4,
        ),
        amd64_digest_tag in "[a-f0-9]{8}",
        insert_at in 0usize..5,
    ) {
        let amd64_digest = format!("sha256:{amd64_digest_tag:0<64}");

        let mut descriptors: Vec<Descriptor> = other_platforms
            .iter()
            .map(|(arch, os)| make_descriptor_with_platform(
                "application/vnd.docker.distribution.manifest.v2+json",
                &format!("sha256:{arch}{os:0<56}"),
                528,
                arch,
                os,
            ))
            .collect();

        // Insert linux/amd64 at the specified position.
        let insert_pos = insert_at.min(descriptors.len());
        descriptors.insert(
            insert_pos,
            make_descriptor_with_platform(
                "application/vnd.docker.distribution.manifest.v2+json",
                &amd64_digest,
                528,
                "amd64",
                "linux",
            ),
        );

        let list = ManifestList {
            schema_version: 2,
            media_type: "application/vnd.docker.distribution.manifest.list.v2+json".to_string(),
            manifests: descriptors,
        };

        let found = list.find_linux_amd64();
        prop_assert!(found.is_some(), "expected to find linux/amd64 entry");
        prop_assert_eq!(&found.unwrap().digest, &amd64_digest);
    }

    /// find_linux_amd64 returns None when no linux/amd64 entry is present.
    #[test]
    fn manifest_list_find_linux_amd64_absent_when_not_present(
        platforms in prop::collection::vec(
            prop_oneof![
                Just(("arm64", "linux")),
                Just(("amd64", "windows")),
                Just(("arm64", "windows")),
                Just(("s390x", "linux")),
            ],
            0..4,
        ),
    ) {
        // Guarantee no linux/amd64 in the list.
        let descriptors: Vec<Descriptor> = platforms
            .iter()
            .map(|(arch, os)| make_descriptor_with_platform(
                "application/vnd.docker.distribution.manifest.v2+json",
                &format!("sha256:{arch}{os:0<56}"),
                528,
                arch,
                os,
            ))
            .collect();

        let list = ManifestList {
            schema_version: 2,
            media_type: "application/vnd.docker.distribution.manifest.list.v2+json".to_string(),
            manifests: descriptors,
        };

        prop_assert!(list.find_linux_amd64().is_none());
    }

    // -----------------------------------------------------------------------
    // Manifest parsing — invalid / malformed input
    // -----------------------------------------------------------------------

    /// parse rejects arbitrary non-JSON bytes.
    #[test]
    fn parse_rejects_non_json_bytes(
        // Restrict to bytes that can't accidentally form valid JSON.
        garbage in prop::collection::vec(0x01u8..0x20u8, 1..256)
    ) {
        let result = ManifestResponse::parse(
            &garbage,
            "application/vnd.oci.image.manifest.v1+json",
        );
        prop_assert!(result.is_err(), "expected Err for non-JSON input");
    }

    /// parse rejects empty byte slice.
    #[test]
    fn parse_rejects_empty_body(
        media_type in prop_oneof![
            Just("application/vnd.oci.image.manifest.v1+json"),
            Just("application/vnd.docker.distribution.manifest.list.v2+json"),
            Just("application/vnd.oci.image.index.v1+json"),
        ]
    ) {
        let result = ManifestResponse::parse(b"", media_type);
        prop_assert!(result.is_err(), "expected Err for empty body");
    }
}

// ---------------------------------------------------------------------------
// Non-proptest sanity checks (round-trip OciManifest via serde)
// ---------------------------------------------------------------------------

#[test]
fn oci_manifest_serde_roundtrip() {
    let manifest = OciManifest {
        schema_version: 2,
        media_type: "application/vnd.oci.image.manifest.v1+json".to_string(),
        config: make_descriptor(
            "application/vnd.oci.image.config.v1+json",
            "sha256:configdigest000000000000000000000000000000000000000000000000000000",
            512,
        ),
        layers: vec![make_descriptor(
            "application/vnd.oci.image.layer.v1.tar+gzip",
            "sha256:layerdigest0000000000000000000000000000000000000000000000000000000",
            4096,
        )],
    };

    let json = serde_json::to_vec(&manifest).unwrap();
    let parsed = ManifestResponse::parse(&json, "application/vnd.oci.image.manifest.v1+json")
        .expect("roundtrip parse failed");

    match parsed {
        ManifestResponse::Single(m) => {
            assert_eq!(m.schema_version, 2);
            assert_eq!(m.layers.len(), 1);
        }
        ManifestResponse::List(_) => panic!("expected Single"),
    }
}

#[test]
fn verify_digest_empty_data() {
    // SHA256 of empty slice is well-defined.
    let digest = format!("sha256:{}", sha256_hex(&[]));
    verify_digest(&[], &digest).expect("empty data with correct digest should pass");
}
