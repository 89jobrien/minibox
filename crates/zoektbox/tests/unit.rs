use sha2::Digest as _;
use zoektbox::download::verify_sha256;
use zoektbox::release::{ZoektPlatform, expected_sha256, release_url};

#[test]
fn release_url_linux_amd64_contains_version_and_triple() {
    let url = release_url(ZoektPlatform::LinuxAmd64);
    assert!(url.contains("linux_amd64"), "url: {url}");
    assert!(url.contains(zoektbox::ZOEKT_VERSION), "url: {url}");
    assert!(
        url.starts_with("https://github.com/sourcegraph/zoekt/releases/"),
        "url: {url}"
    );
}

#[test]
fn release_url_differs_per_platform() {
    let a = release_url(ZoektPlatform::LinuxAmd64);
    let b = release_url(ZoektPlatform::LinuxArm64);
    let c = release_url(ZoektPlatform::DarwinArm64);
    assert_ne!(a, b);
    assert_ne!(b, c);
}

#[test]
fn expected_sha256_is_64_hex_chars() {
    for platform in [
        ZoektPlatform::LinuxAmd64,
        ZoektPlatform::LinuxArm64,
        ZoektPlatform::DarwinArm64,
    ] {
        let digest = expected_sha256(platform);
        assert_eq!(
            digest.len(),
            64,
            "platform {platform:?}: len={}",
            digest.len()
        );
        assert!(
            digest.chars().all(|c| c.is_ascii_hexdigit()),
            "not hex: {digest}"
        );
    }
}

#[test]
fn verify_sha256_passes_on_correct_digest() {
    let data = b"hello zoekt";
    let digest = hex::encode(sha2::Sha256::digest(data));
    verify_sha256(data, &digest).expect("should pass");
}

#[test]
fn verify_sha256_fails_on_wrong_digest() {
    let data = b"hello zoekt";
    let err = verify_sha256(
        data,
        "deadbeef00000000000000000000000000000000000000000000000000000000",
    );
    assert!(err.is_err());
    let msg = err.unwrap_err().to_string();
    assert!(msg.contains("SHA256 mismatch"), "msg: {msg}");
}

#[test]
#[ignore = "fill in real SHA-256 checksums in release.rs before enabling"]
fn expected_sha256_is_not_placeholder() {
    for platform in [
        ZoektPlatform::LinuxAmd64,
        ZoektPlatform::LinuxArm64,
        ZoektPlatform::DarwinArm64,
    ] {
        let digest = expected_sha256(platform);
        assert_ne!(
            digest, "0000000000000000000000000000000000000000000000000000000000000000",
            "platform {platform:?}: placeholder SHA not replaced"
        );
    }
}
