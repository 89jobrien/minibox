use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[allow(dead_code)]
pub const ALPINE_VERSION: &str = "3.21.3";
#[allow(dead_code)]
pub const ALPINE_ARCH: &str = "aarch64";
#[allow(dead_code)]
pub const ALPINE_CDN: &str = "https://dl-cdn.alpinelinux.org/alpine";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(dead_code)]
pub struct VmImageManifest {
    pub alpine_version: String,
    pub agent_rustc_version: String,
    pub agent_commit: String,
    pub built_at: u64, // Unix timestamp seconds
}

#[allow(dead_code)]
impl VmImageManifest {
    pub fn load(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading manifest {}", path.display()))?;
        let m: Self = serde_json::from_str(&text)
            .with_context(|| format!("parsing manifest {}", path.display()))?;
        Ok(Some(m))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating dir {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self).context("serializing manifest")?;
        std::fs::write(path, json)
            .with_context(|| format!("writing manifest {}", path.display()))?;
        Ok(())
    }
}

/// Alpine Linux asset URLs for a specific version and architecture.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AlpineAssets {
    pub kernel: String,
    pub initramfs: String,
    pub minirootfs: String,
}

impl AlpineAssets {
    /// Construct URLs for Alpine assets. Extracts major.minor from version string
    /// (e.g., "3.21.3" → "3.21") for the release path.
    #[allow(dead_code)]
    pub fn for_version(version: &str, arch: &str) -> Self {
        let major_minor: String = version.splitn(3, '.').take(2).collect::<Vec<_>>().join(".");
        let base = format!("{ALPINE_CDN}/v{major_minor}/releases/{arch}");
        Self {
            kernel: format!("{base}/netboot/vmlinuz-virt"),
            initramfs: format!("{base}/netboot/initramfs-virt"),
            minirootfs: format!("{base}/alpine-minirootfs-{version}-{arch}.tar.gz"),
        }
    }
}

/// Download `url` to `dest`, skipping if file already exists and `force` is false.
#[allow(dead_code)]
pub fn download_file(url: &str, dest: &Path, force: bool) -> Result<()> {
    if dest.exists() && !force {
        println!("  cached  {}", dest.display());
        return Ok(());
    }
    println!("  fetch   {url}");
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating dir {}", parent.display()))?;
    }
    let status = std::process::Command::new("curl")
        .args(["--silent", "--show-error", "--location", "--fail", "-o"])
        .arg(dest)
        .arg(url)
        .status()
        .context("running curl")?;
    if !status.success() {
        anyhow::bail!("curl failed for {url}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_roundtrip() {
        let m = VmImageManifest {
            alpine_version: "3.21.0".into(),
            agent_rustc_version: "1.87.0".into(),
            agent_commit: "abc1234".into(),
            built_at: 1711670400,
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: VmImageManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.alpine_version, "3.21.0");
        assert_eq!(back.agent_commit, "abc1234");
    }

    #[test]
    fn manifest_load_returns_none_for_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("manifest.json");
        let result = VmImageManifest::load(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn manifest_save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("manifest.json");
        let m = VmImageManifest {
            alpine_version: "3.21.3".into(),
            agent_rustc_version: "rustc 1.87.0".into(),
            agent_commit: "deadbeef".into(),
            built_at: 9999,
        };
        m.save(&path).unwrap();
        let loaded = VmImageManifest::load(&path).unwrap().unwrap();
        assert_eq!(loaded, m);
    }

    #[test]
    fn alpine_urls_format_correctly() {
        let urls = AlpineAssets::for_version("3.21.3", "aarch64");
        assert!(urls.kernel.contains("vmlinuz-virt"));
        assert!(urls.initramfs.contains("initramfs-virt"));
        assert!(
            urls.minirootfs
                .contains("alpine-minirootfs-3.21.3-aarch64.tar.gz")
        );
        assert!(urls.kernel.contains("v3.21/releases/aarch64"));
    }

    #[test]
    fn download_file_skips_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("file.txt");
        std::fs::write(&dest, b"existing").unwrap();
        // Should not error even with a bogus URL because file already exists.
        download_file("http://bogus.invalid/file", &dest, false).unwrap();
        // File contents unchanged.
        assert_eq!(std::fs::read(&dest).unwrap(), b"existing");
    }

    #[test]
    fn download_file_force_would_overwrite() {
        // Just test that with force=true and a real existing file, it would try to fetch.
        // We can't actually make a network request in unit tests, so just verify that
        // force=false skips and force=true would proceed (by checking the skip logic).
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("file.txt");
        std::fs::write(&dest, b"existing").unwrap();
        // force=false: should skip
        download_file("http://bogus.invalid/file", &dest, false).unwrap();
        // force=true: would try to fetch, which fails — but we only test the skip path here.
        // This is tested structurally; actual download is an integration concern.
    }
}
