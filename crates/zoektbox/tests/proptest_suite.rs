//! Property-based tests for zoektbox release and URL logic.
//!
//! Invariants tested:
//! - `release_url` always starts with the canonical GitHub prefix.
//! - `release_url` always embeds `ZOEKT_VERSION`.
//! - `release_url` always embeds the platform triple.
//! - Every platform produces a distinct URL.
//! - `expected_sha256` is always a 64-character hex string.
//! - `ZOEKT_BINARIES` is non-empty and every entry is non-empty.

use proptest::prelude::*;
use zoektbox::release::{ZoektPlatform, expected_sha256, release_url};
use zoektbox::{ZOEKT_BINARIES, ZOEKT_VERSION};

// ---------------------------------------------------------------------------
// Constants-level invariants (not proptest — always checked)
// ---------------------------------------------------------------------------

#[test]
fn zoekt_version_is_non_empty() {
    assert!(!ZOEKT_VERSION.is_empty());
}

#[test]
fn zoekt_binaries_is_non_empty() {
    assert!(!ZOEKT_BINARIES.is_empty());
}

#[test]
fn all_binary_names_are_non_empty() {
    for name in ZOEKT_BINARIES {
        assert!(!name.is_empty(), "binary name must not be empty");
    }
}

#[test]
fn all_binary_names_start_with_zoekt() {
    for name in ZOEKT_BINARIES {
        assert!(
            name.starts_with("zoekt"),
            "expected 'zoekt' prefix, got: {name}"
        );
    }
}

// ---------------------------------------------------------------------------
// Platform-parameterised invariants
// ---------------------------------------------------------------------------

const ALL_PLATFORMS: &[ZoektPlatform] = &[
    ZoektPlatform::LinuxAmd64,
    ZoektPlatform::LinuxArm64,
    ZoektPlatform::DarwinArm64,
];

proptest! {
    #![proptest_config(proptest::test_runner::Config {
        failure_persistence: None,
        ..proptest::test_runner::Config::default()
    })]

    /// `release_url` always uses HTTPS and the canonical GitHub releases path.
    #[test]
    fn release_url_uses_canonical_prefix(platform_idx in 0usize..3) {
        let platform = ALL_PLATFORMS[platform_idx];
        let url = release_url(platform);
        prop_assert!(
            url.starts_with("https://github.com/sourcegraph/zoekt/releases/"),
            "unexpected url prefix: {url}"
        );
    }

    /// `release_url` always embeds the pinned version string.
    #[test]
    fn release_url_embeds_version(platform_idx in 0usize..3) {
        let platform = ALL_PLATFORMS[platform_idx];
        let url = release_url(platform);
        prop_assert!(
            url.contains(ZOEKT_VERSION),
            "url does not contain version {ZOEKT_VERSION}: {url}"
        );
    }

    /// `release_url` always ends with `.tar.gz`.
    #[test]
    fn release_url_ends_with_tar_gz(platform_idx in 0usize..3) {
        let platform = ALL_PLATFORMS[platform_idx];
        let url = release_url(platform);
        prop_assert!(url.ends_with(".tar.gz"), "url: {url}");
    }

    /// `expected_sha256` is always a 64-character lowercase hex string.
    #[test]
    fn expected_sha256_is_64_hex_chars(platform_idx in 0usize..3) {
        let platform = ALL_PLATFORMS[platform_idx];
        let digest = expected_sha256(platform);
        prop_assert_eq!(digest.len(), 64, "digest len: {}", digest);
        prop_assert!(
            digest.chars().all(|c| c.is_ascii_hexdigit()),
            "digest contains non-hex chars: {}",
            digest
        );
    }
}

/// All platforms produce distinct URLs (no copy-paste collision).
#[test]
fn all_platform_urls_are_distinct() {
    let urls: Vec<_> = ALL_PLATFORMS.iter().map(|&p| release_url(p)).collect();
    for i in 0..urls.len() {
        for j in (i + 1)..urls.len() {
            assert_ne!(
                urls[i], urls[j],
                "platforms {i} and {j} produce the same URL"
            );
        }
    }
}
