use zoektbox::release::{expected_sha256, release_url, ZoektPlatform};

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
        assert_eq!(digest.len(), 64, "platform {platform:?}: len={}", digest.len());
        assert!(
            digest.chars().all(|c| c.is_ascii_hexdigit()),
            "not hex: {digest}"
        );
    }
}
