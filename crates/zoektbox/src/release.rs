//! Pinned Zoekt release manifest. Update `ZOEKT_VERSION` and checksums on each upgrade.

pub const ZOEKT_VERSION: &str = "3.7.2-89.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoektPlatform {
    LinuxAmd64,
    LinuxArm64,
    DarwinArm64,
}

impl ZoektPlatform {
    pub fn detect() -> anyhow::Result<Self> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => Ok(Self::LinuxAmd64),
            ("linux", "aarch64") => Ok(Self::LinuxArm64),
            ("macos", "aarch64") => Ok(Self::DarwinArm64),
            (os, arch) => anyhow::bail!("unsupported platform: {os}/{arch}"),
        }
    }

    fn triple(&self) -> &'static str {
        match self {
            Self::LinuxAmd64 => "linux_amd64",
            Self::LinuxArm64 => "linux_arm64",
            Self::DarwinArm64 => "darwin_arm64",
        }
    }
}

/// GitHub release tarball URL for the given platform.
pub fn release_url(platform: ZoektPlatform) -> String {
    format!(
        "https://github.com/sourcegraph/zoekt/releases/download/v{version}/zoekt_{version}_{triple}.tar.gz",
        version = ZOEKT_VERSION,
        triple = platform.triple(),
    )
}

/// Expected SHA256 hex digest for each platform's tarball.
/// Run `sha256sum <tarball>` after downloading to verify and update these.
pub fn expected_sha256(platform: ZoektPlatform) -> &'static str {
    match platform {
        // TODO: fill in after first download — run `sha256sum` on each tarball
        ZoektPlatform::LinuxAmd64 => {
            "0000000000000000000000000000000000000000000000000000000000000000"
        }
        ZoektPlatform::LinuxArm64 => {
            "0000000000000000000000000000000000000000000000000000000000000000"
        }
        ZoektPlatform::DarwinArm64 => {
            "0000000000000000000000000000000000000000000000000000000000000000"
        }
    }
}

/// Names of binaries extracted from the tarball.
pub const ZOEKT_BINARIES: &[&str] = &[
    "zoekt-webserver",
    "zoekt-indexserver",
    "zoekt-git-index",
    "zoekt-index",
];
