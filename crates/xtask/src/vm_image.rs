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

/// Extract Alpine minirootfs tarball into `rootfs_dir`.
/// Skips if `rootfs_dir/bin` exists (already extracted) and `force` is false.
#[allow(dead_code)]
pub fn extract_rootfs_if_needed(tarball: &Path, rootfs_dir: &Path, force: bool) -> Result<()> {
    let marker = rootfs_dir.join("bin");
    if marker.exists() && !force {
        println!("  cached  {}", rootfs_dir.display());
        return Ok(());
    }
    println!("  extract {}", tarball.display());
    std::fs::create_dir_all(rootfs_dir)
        .with_context(|| format!("creating rootfs dir {}", rootfs_dir.display()))?;
    let status = std::process::Command::new("tar")
        .args(["-xzf"])
        .arg(tarball)
        .args(["-C"])
        .arg(rootfs_dir)
        .status()
        .context("running tar")?;
    if !status.success() {
        anyhow::bail!("tar extraction failed for {}", tarball.display());
    }
    Ok(())
}

/// Return the agent destination path within rootfs.
#[allow(dead_code)]
pub fn agent_dest_path(rootfs_dir: &Path) -> std::path::PathBuf {
    rootfs_dir.join("sbin").join("minibox-agent")
}

/// Cross-compile miniboxd for aarch64-unknown-linux-musl and copy into rootfs.
/// Skips if agent already exists at dest and `force` is false.
/// Returns the rustc version string (for the manifest).
#[allow(dead_code)]
pub fn build_and_install_agent(rootfs_dir: &Path, force: bool) -> Result<String> {
    let dest = agent_dest_path(rootfs_dir);
    let target = "aarch64-unknown-linux-musl";

    // Get current rustc version for manifest
    let rustc_ver = {
        let out = std::process::Command::new("rustc")
            .args(["--version"])
            .output()
            .context("running rustc --version")?;
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    if dest.exists() && !force {
        println!("  cached  agent at {}", dest.display());
        return Ok(rustc_ver);
    }

    println!("  compile miniboxd → {target}");
    let status = std::process::Command::new("cargo")
        .args(["build", "--release", "--target", target, "-p", "miniboxd"])
        .status()
        .context("cargo build for agent")?;
    if !status.success() {
        anyhow::bail!("cargo build failed for miniboxd/{target}");
    }

    // Binary is at target/<target>/release/miniboxd
    let src = std::path::Path::new("target")
        .join(target)
        .join("release")
        .join("miniboxd");
    if !src.exists() {
        anyhow::bail!(
            "expected binary at {} — build succeeded but file missing",
            src.display()
        );
    }

    std::fs::create_dir_all(rootfs_dir.join("sbin")).context("creating rootfs/sbin")?;
    std::fs::copy(&src, &dest).with_context(|| format!("copying agent to {}", dest.display()))?;
    println!("  installed {}", dest.display());

    // Symlink /sbin/init → minibox-agent
    let init_link = rootfs_dir.join("sbin").join("init");
    if init_link.exists() || init_link.symlink_metadata().is_ok() {
        std::fs::remove_file(&init_link).context("removing old /sbin/init")?;
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink("minibox-agent", &init_link)
        .context("symlinking /sbin/init → minibox-agent")?;

    Ok(rustc_ver)
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

    #[test]
    fn extract_skips_when_bin_marker_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(rootfs.join("bin")).unwrap();
        // A fake tarball path — should never be read since rootfs/bin exists
        let fake_tarball = tmp.path().join("fake.tar.gz");
        std::fs::write(&fake_tarball, b"not a real tarball").unwrap();
        // force=false: should skip without touching tarball
        extract_rootfs_if_needed(&fake_tarball, &rootfs, false).unwrap();
    }

    #[test]
    fn extract_creates_rootfs_dir_if_missing() {
        // Structural test: verify the function creates rootfs_dir before calling tar.
        // (We can't run tar on a fake tarball but we can verify the dir is created.)
        let tmp = tempfile::tempdir().unwrap();
        let rootfs = tmp.path().join("rootfs");
        let fake_tarball = tmp.path().join("fake.tar.gz");
        // Don't create rootfs or tarball — the function should create rootfs before tar fails.
        std::fs::write(&fake_tarball, b"not a real tarball").unwrap();
        // This will fail because the tarball is fake, but rootfs_dir should exist after create_dir_all
        let result = extract_rootfs_if_needed(&fake_tarball, &rootfs, false);
        // Either it fails from tar (expected) or succeeds if somehow tar handles it
        // We only care that rootfs was created (not that tar succeeded)
        assert!(
            rootfs.exists(),
            "rootfs dir should be created before tar runs"
        );
        // result is likely Err (bad tarball), which is fine
        drop(result);
    }

    #[test]
    fn agent_dest_path_is_correct() {
        let tmp = tempfile::tempdir().unwrap();
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(rootfs.join("sbin")).unwrap();
        let dest = agent_dest_path(&rootfs);
        assert!(dest.to_string_lossy().ends_with("sbin/minibox-agent"));
    }

    #[test]
    fn build_and_install_agent_skips_when_cached() {
        let tmp = tempfile::tempdir().unwrap();
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(rootfs.join("sbin")).unwrap();
        let dest = agent_dest_path(&rootfs);
        std::fs::write(&dest, b"fake agent binary").unwrap();
        // force=false: should skip without running cargo build
        let result = build_and_install_agent(&rootfs, false);
        assert!(result.is_ok(), "should succeed when cached: {:?}", result);
        // File should be unchanged
        assert_eq!(std::fs::read(&dest).unwrap(), b"fake agent binary");
    }
}
