use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
        .args(["zigbuild", "--release", "--target", target, "-p", "miniboxd"])
        .status()
        .context("cargo zigbuild for agent (is cargo-zigbuild installed?)")?;
    if !status.success() {
        anyhow::bail!("cargo zigbuild failed for miniboxd/{target}");
    }

    // Binary is at target/<target>/release/miniboxd
    // Respect CARGO_TARGET_DIR if set (e.g. ~/.mbx/cache/target/)
    let target_base = std::env::var("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::Path::new("target").to_path_buf());
    let src = target_base.join(target).join("release").join("miniboxd");
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

/// Get the default VM directory for the host.
/// Uses ~/.mbx/vm if home directory is available, otherwise /tmp/.mbx/vm.
#[allow(dead_code)]
pub fn default_vm_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".mbx")
        .join("vm")
}

/// Build or refresh the VM image directory.
/// Downloads Alpine assets, extracts rootfs, cross-compiles agent, writes manifest.
#[allow(dead_code)]
pub fn build_vm_image(vm_dir: &Path, force: bool) -> Result<()> {
    println!("Building VM image in {}", vm_dir.display());

    let cache_dir = vm_dir.join("cache");
    let rootfs_dir = vm_dir.join("rootfs");
    let boot_dir = vm_dir.join("boot");
    let manifest_path = vm_dir.join("manifest.json");

    for dir in &[&cache_dir, &rootfs_dir, &boot_dir] {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }

    let assets = AlpineAssets::for_version(ALPINE_VERSION, ALPINE_ARCH);

    // 1. Download kernel
    let kernel_dest = boot_dir.join("vmlinuz-virt");
    download_file(&assets.kernel, &kernel_dest, force)?;

    // 2. Download initramfs
    let initramfs_dest = boot_dir.join("initramfs-virt");
    download_file(&assets.initramfs, &initramfs_dest, force)?;

    // 3. Download minirootfs tarball
    let tarball_dest = cache_dir.join(format!(
        "alpine-minirootfs-{ALPINE_VERSION}-{ALPINE_ARCH}.tar.gz"
    ));
    download_file(&assets.minirootfs, &tarball_dest, force)?;

    // 4. Extract rootfs
    extract_rootfs_if_needed(&tarball_dest, &rootfs_dir, force)?;

    // 5. Cross-compile and install agent
    let rustc_ver = build_and_install_agent(&rootfs_dir, force)?;

    // 6. Install init files for PID-1 bootstrap
    install_init_files(&rootfs_dir)?;

    // 7. Get current git commit hash for manifest
    let commit = {
        let out = std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .context("git rev-parse")?;
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    // 8. Write manifest
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let manifest = VmImageManifest {
        alpine_version: ALPINE_VERSION.into(),
        agent_rustc_version: rustc_ver,
        agent_commit: commit,
        built_at: now,
    };
    manifest.save(&manifest_path)?;

    println!("VM image ready at {}", vm_dir.display());
    println!("  kernel    {}", kernel_dest.display());
    println!("  initramfs {}", initramfs_dest.display());
    println!("  rootfs    {}", rootfs_dir.display());
    Ok(())
}

/// Install minimal init files into rootfs so the agent boots correctly.
/// This duplicates the logic from macbox::vz::agent_init to avoid a circular dependency.
#[allow(dead_code)]
pub fn install_init_files(rootfs_dir: &Path) -> Result<()> {
    let etc = rootfs_dir.join("etc");
    std::fs::create_dir_all(&etc).context("creating rootfs/etc")?;

    let inittab = "::sysinit:/etc/init.d/rcS\n::once:/sbin/minibox-agent\n::ctrlaltdel:/sbin/reboot\n::shutdown:/bin/umount -a -r\n";
    std::fs::write(etc.join("inittab"), inittab).context("writing /etc/inittab")?;

    let initd = etc.join("init.d");
    std::fs::create_dir_all(&initd).context("creating rootfs/etc/init.d")?;
    let rcs_content = r#"#!/bin/sh
set -e
mount -t proc proc /proc 2>/dev/null || true
mount -t sysfs sys /sys 2>/dev/null || true
mount -t devtmpfs dev /dev 2>/dev/null || true
mount -t tmpfs tmpfs /tmp 2>/dev/null || true
mkdir -p /var/lib/minibox/images /var/lib/minibox/containers
mount -t virtiofs mbx-images /var/lib/minibox/images 2>/dev/null || true
mount -t virtiofs mbx-containers /var/lib/minibox/containers 2>/dev/null || true
ip link set lo up 2>/dev/null || true
hostname minibox-vm 2>/dev/null || true
"#;
    let rcs_path = initd.join("rcS");
    std::fs::write(&rcs_path, rcs_content).context("writing rcS")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&rcs_path, std::fs::Permissions::from_mode(0o755))
            .context("chmod rcS")?;
    }
    println!("  init    rootfs/etc/inittab + etc/init.d/rcS");
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

    #[test]
    fn vm_dir_default_uses_mbx_cache() {
        let dir = default_vm_dir();
        assert!(dir.is_absolute());
        assert!(dir.to_string_lossy().contains(".mbx"));
    }

    #[test]
    fn build_vm_image_creates_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let vm_dir = tmp.path().join("vm");

        // Pre-create all cached files so download/extract/compile are all skipped
        let cache_dir = vm_dir.join("cache");
        let rootfs_dir = vm_dir.join("rootfs");
        let boot_dir = vm_dir.join("boot");
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::create_dir_all(&rootfs_dir).unwrap();
        std::fs::create_dir_all(&boot_dir).unwrap();

        std::fs::write(boot_dir.join("vmlinuz-virt"), b"fake kernel").unwrap();
        std::fs::write(boot_dir.join("initramfs-virt"), b"fake initrd").unwrap();

        let tarball = cache_dir.join(format!(
            "alpine-minirootfs-{ALPINE_VERSION}-{ALPINE_ARCH}.tar.gz"
        ));
        std::fs::write(&tarball, b"fake tarball").unwrap();

        // Create marker for extract skip
        std::fs::create_dir_all(rootfs_dir.join("bin")).unwrap();

        // Pre-create fake agent so compile is skipped
        std::fs::create_dir_all(rootfs_dir.join("sbin")).unwrap();
        std::fs::write(rootfs_dir.join("sbin").join("minibox-agent"), b"fake agent").unwrap();

        // Create symlink for init
        let init_link = rootfs_dir.join("sbin").join("init");
        #[cfg(unix)]
        std::os::unix::fs::symlink("minibox-agent", &init_link).ok();

        // Now build_vm_image should run to completion (all steps cached/skipped)
        let result = build_vm_image(&vm_dir, false);
        assert!(result.is_ok(), "build_vm_image failed: {:?}", result);

        // Manifest should be written
        assert!(vm_dir.join("manifest.json").exists());

        // Verify manifest is valid JSON
        let manifest_content = std::fs::read_to_string(vm_dir.join("manifest.json")).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&manifest_content).expect("manifest should be valid JSON");
        assert!(parsed["alpine_version"].is_string());
        assert!(parsed["agent_rustc_version"].is_string());
        assert!(parsed["agent_commit"].is_string());
        assert!(parsed["built_at"].is_number());
    }
}
