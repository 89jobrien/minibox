# macOS VZ.framework — VM Image Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an xtask pipeline that downloads an Alpine aarch64 kernel + initrd + minirootfs, cross-compiles minibox-agent, and assembles a ready-to-boot VM image directory that `macbox` can use to launch a Linux VM via Apple's Virtualization.framework.

**Architecture:** The `build-vm-image` xtask command downloads Alpine Linux aarch64 assets (kernel `vmlinuz-virt`, `initramfs-virt`, minirootfs tarball) from the Alpine CDN, extracts the rootfs to `~/.mbx/vm/rootfs/`, cross-compiles `miniboxd` for `aarch64-unknown-linux-musl` and places it at `rootfs/sbin/minibox-agent`, then writes a `VmImageManifest` JSON to `~/.mbx/vm/manifest.json` for cache invalidation. A new `vz` feature flag in `macbox` gates the VZ code path; `macbox` reads the manifest, checks staleness, optionally re-runs `build-vm-image`, then boots the VM using VZ.framework via thin `objc2` bindings.

**Tech Stack:** Rust (xtask + xshell), `aarch64-unknown-linux-musl` cross target (already in `.cargo/config.toml`), Alpine Linux CDN, `objc2` crate (Apple framework bindings), Tokio `spawn_blocking` for GCD calls.

---

## File Map

| Action | Path                               | Responsibility                                          |
| ------ | ---------------------------------- | ------------------------------------------------------- |
| Modify | `crates/xtask/src/main.rs`         | Add `build-vm-image` dispatch + `build_vm_image()` fn   |
| Create | `crates/xtask/src/vm_image.rs`     | All download/extract/cross-compile logic                |
| Modify | `crates/macbox/Cargo.toml`         | Add `objc2`, `objc2-foundation` deps; `vz` feature flag |
| Create | `crates/macbox/src/vz/mod.rs`      | Public re-exports for VZ bindings module                |
| Create | `crates/macbox/src/vz/bindings.rs` | Thin `objc2` wrappers for VZ.framework classes          |
| Create | `crates/macbox/src/vz/vm.rs`       | `VzVm` struct: boot, wait-ready, shutdown               |
| Modify | `crates/macbox/src/lib.rs`         | `mod vz` behind `#[cfg(feature = "vz")]`                |
| Create | `~/.mbx/vm/manifest.json`          | **Runtime artifact** — versioned image manifest         |

---

### Task 1: `VmImageManifest` type and serialization

**Files:**

- Create: `crates/xtask/src/vm_image.rs`

- [ ] **Step 1: Write the failing test**

```rust
// crates/xtask/src/vm_image.rs
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
}
```

- [ ] **Step 2: Run test to confirm it fails**

```
cargo test -p xtask vm_image::tests::manifest_roundtrip
```

Expected: compile error — `vm_image` module does not exist yet.

- [ ] **Step 3: Implement the type**

```rust
// crates/xtask/src/vm_image.rs
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const ALPINE_VERSION: &str = "3.21.3";
pub const ALPINE_ARCH: &str = "aarch64";
pub const ALPINE_CDN: &str = "https://dl-cdn.alpinelinux.org/alpine";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VmImageManifest {
    pub alpine_version: String,
    pub agent_rustc_version: String,
    pub agent_commit: String,
    pub built_at: u64, // Unix timestamp seconds
}

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
```

Wire it into `crates/xtask/src/main.rs`:

```rust
mod vm_image;
```

- [ ] **Step 4: Run test to confirm it passes**

```
cargo test -p xtask vm_image::tests::manifest_roundtrip
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/xtask/src/vm_image.rs crates/xtask/src/main.rs
git commit -m "feat(xtask): add VmImageManifest type with load/save"
```

---

### Task 2: Alpine asset download helpers

**Files:**

- Modify: `crates/xtask/src/vm_image.rs`

The Alpine virt kernel, initramfs, and minirootfs tarball are downloaded from:

- `https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/aarch64/alpine-virt-3.21.3-aarch64.iso` — NO, we need individual files:
  - kernel: `https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/aarch64/netboot/vmlinuz-virt`
  - initramfs: `https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/aarch64/netboot/initramfs-virt`
  - minirootfs: `https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/aarch64/alpine-minirootfs-3.21.3-aarch64.tar.gz`

- [ ] **Step 1: Write the failing test**

```rust
// Add to crates/xtask/src/vm_image.rs tests module
#[test]
fn alpine_urls_format_correctly() {
    let urls = AlpineAssets::for_version("3.21.3", "aarch64");
    assert!(urls.kernel.contains("vmlinuz-virt"));
    assert!(urls.initramfs.contains("initramfs-virt"));
    assert!(urls.minirootfs.contains("alpine-minirootfs-3.21.3-aarch64.tar.gz"));
}
```

- [ ] **Step 2: Run test to confirm it fails**

```
cargo test -p xtask alpine_urls
```

Expected: compile error — `AlpineAssets` not defined.

- [ ] **Step 3: Implement**

```rust
// Add to crates/xtask/src/vm_image.rs (after VmImageManifest)

pub struct AlpineAssets {
    pub kernel: String,
    pub initramfs: String,
    pub minirootfs: String,
}

impl AlpineAssets {
    pub fn for_version(version: &str, arch: &str) -> Self {
        // major.minor only for the path component (e.g. 3.21)
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
    let status = Command::new("curl")
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
```

(Add `use std::process::Command;` — already imported in `main.rs` scope but `vm_image.rs` needs its own.)

- [ ] **Step 4: Run test to confirm it passes**

```
cargo test -p xtask alpine_urls
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/xtask/src/vm_image.rs
git commit -m "feat(xtask): AlpineAssets URL builder and download_file helper"
```

---

### Task 3: Rootfs extraction

**Files:**

- Modify: `crates/xtask/src/vm_image.rs`

The minirootfs tarball is extracted to `vm_dir/rootfs/`. We use the host `tar` command — no Rust tar crate in xtask to keep compile times low.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn extract_skips_when_rootfs_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let rootfs = tmp.path().join("rootfs");
    std::fs::create_dir_all(&rootfs).unwrap();
    // If rootfs dir exists and we're not forcing, extract_rootfs returns Ok early.
    // We verify by checking the function doesn't error even with a fake tarball path.
    let fake_tarball = tmp.path().join("fake.tar.gz");
    std::fs::write(&fake_tarball, b"").unwrap();
    // Should return Ok without touching the tarball.
    extract_rootfs_if_needed(&fake_tarball, &rootfs, false).unwrap();
}
```

- [ ] **Step 2: Run test to confirm it fails**

```
cargo test -p xtask extract_skips
```

Expected: compile error.

- [ ] **Step 3: Implement**

```rust
// Add to crates/xtask/src/vm_image.rs

/// Extract Alpine minirootfs tarball into `rootfs_dir`.
/// Skips if `rootfs_dir/bin` exists (already extracted) and `force` is false.
pub fn extract_rootfs_if_needed(tarball: &Path, rootfs_dir: &Path, force: bool) -> Result<()> {
    let marker = rootfs_dir.join("bin");
    if marker.exists() && !force {
        println!("  cached  {}", rootfs_dir.display());
        return Ok(());
    }
    println!("  extract {}", tarball.display());
    std::fs::create_dir_all(rootfs_dir)
        .with_context(|| format!("creating rootfs dir {}", rootfs_dir.display()))?;
    let status = Command::new("tar")
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
```

Add `tempfile` to xtask dev-dependencies (`Cargo.toml`):

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 4: Run test to confirm it passes**

```
cargo test -p xtask extract_skips
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/xtask/src/vm_image.rs crates/xtask/Cargo.toml
git commit -m "feat(xtask): rootfs extraction helper with skip-if-cached"
```

---

### Task 4: Cross-compile minibox-agent and place into rootfs

**Files:**

- Modify: `crates/xtask/src/vm_image.rs`

The agent IS `miniboxd` compiled for `aarch64-unknown-linux-musl`. It goes to `rootfs/sbin/minibox-agent`. We also symlink `rootfs/sbin/init -> minibox-agent` so VZ.framework can boot with it as PID 1.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn agent_dest_path_is_correct() {
    let tmp = tempfile::tempdir().unwrap();
    let rootfs = tmp.path().join("rootfs");
    std::fs::create_dir_all(rootfs.join("sbin")).unwrap();
    let dest = agent_dest_path(&rootfs);
    assert!(dest.ends_with("sbin/minibox-agent"));
}
```

- [ ] **Step 2: Run test to confirm it fails**

```
cargo test -p xtask agent_dest_path
```

Expected: compile error.

- [ ] **Step 3: Implement**

```rust
// Add to crates/xtask/src/vm_image.rs

pub fn agent_dest_path(rootfs_dir: &Path) -> std::path::PathBuf {
    rootfs_dir.join("sbin").join("minibox-agent")
}

/// Cross-compile miniboxd for aarch64-unknown-linux-musl and copy into rootfs.
/// Skips if agent already exists at dest and `force` is false.
pub fn build_and_install_agent(rootfs_dir: &Path, force: bool) -> Result<String> {
    let dest = agent_dest_path(rootfs_dir);
    let target = "aarch64-unknown-linux-musl";

    // Get current rustc version for manifest
    let rustc_ver = {
        let out = Command::new("rustc")
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
    let status = Command::new("cargo")
        .args(["build", "--release", "--target", target, "-p", "miniboxd"])
        .status()
        .context("cargo build for agent")?;
    if !status.success() {
        anyhow::bail!("cargo build failed for miniboxd/{target}");
    }

    // Binary is at target/<target>/release/miniboxd
    let src = Path::new("target").join(target).join("release").join("miniboxd");
    if !src.exists() {
        anyhow::bail!("expected binary at {} — build succeeded but file missing", src.display());
    }

    std::fs::create_dir_all(rootfs_dir.join("sbin"))
        .context("creating rootfs/sbin")?;
    std::fs::copy(&src, &dest)
        .with_context(|| format!("copying agent to {}", dest.display()))?;
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
```

- [ ] **Step 4: Run test to confirm it passes**

```
cargo test -p xtask agent_dest_path
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/xtask/src/vm_image.rs
git commit -m "feat(xtask): cross-compile agent and install into rootfs/sbin"
```

---

### Task 5: `build_vm_image` orchestrator function + xtask dispatch

**Files:**

- Modify: `crates/xtask/src/vm_image.rs`
- Modify: `crates/xtask/src/main.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn vm_dir_default_uses_mbx_cache() {
    // Just test the path computation, not the actual build.
    let dir = default_vm_dir();
    // Should end in .mbx/vm or similar; path must be absolute.
    assert!(dir.is_absolute());
    assert!(dir.to_string_lossy().contains(".mbx"));
}
```

- [ ] **Step 2: Run test to confirm it fails**

```
cargo test -p xtask vm_dir_default
```

Expected: compile error.

- [ ] **Step 3: Implement orchestrator**

```rust
// Add to crates/xtask/src/vm_image.rs

pub fn default_vm_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(".mbx")
        .join("vm")
}

/// Build or refresh the VM image directory.
/// `force` re-downloads and recompiles even if cached assets exist.
pub fn build_vm_image(vm_dir: &Path, force: bool) -> Result<()> {
    println!("Building VM image in {}", vm_dir.display());

    let cache_dir = vm_dir.join("cache");
    let rootfs_dir = vm_dir.join("rootfs");
    let boot_dir = vm_dir.join("boot");
    let manifest_path = vm_dir.join("manifest.json");

    for dir in &[&cache_dir, &rootfs_dir, &boot_dir] {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
    }

    let assets = AlpineAssets::for_version(ALPINE_VERSION, ALPINE_ARCH);

    // 1. Download kernel
    let kernel_dest = boot_dir.join("vmlinuz-virt");
    download_file(&assets.kernel, &kernel_dest, force)?;

    // 2. Download initramfs
    let initramfs_dest = boot_dir.join("initramfs-virt");
    download_file(&assets.initramfs, &initramfs_dest, force)?;

    // 3. Download minirootfs tarball
    let tarball_dest = cache_dir.join(format!("alpine-minirootfs-{ALPINE_VERSION}-{ALPINE_ARCH}.tar.gz"));
    download_file(&assets.minirootfs, &tarball_dest, force)?;

    // 4. Extract rootfs
    extract_rootfs_if_needed(&tarball_dest, &rootfs_dir, force)?;

    // 5. Cross-compile and install agent
    let rustc_ver = build_and_install_agent(&rootfs_dir, force)?;

    // 6. Get current git commit hash for manifest
    let commit = {
        let out = Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .context("git rev-parse")?;
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    // 7. Write manifest
    let now = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
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
```

Add `dirs = "5"` to `crates/xtask/Cargo.toml` dependencies.

- [ ] **Step 4: Wire dispatch in `main.rs`**

```rust
// crates/xtask/src/main.rs — add to match block:
Some("build-vm-image") => {
    let force = env::args().any(|a| a == "--force");
    let vm_dir = vm_image::default_vm_dir();
    vm_image::build_vm_image(&vm_dir, force)
}
```

And in the `None =>` help block:

```rust
eprintln!("  build-vm-image   download Alpine kernel/rootfs, cross-compile agent");
```

- [ ] **Step 5: Run test to confirm it passes**

```
cargo test -p xtask vm_dir_default
```

Expected: PASS.

- [ ] **Step 6: Smoke test the xtask dispatch (no actual download)**

```bash
cargo xtask build-vm-image --help 2>&1 | head -5
# Should not error — just prints help for unknown args and exits
cargo check -p xtask
```

Expected: compiles without errors.

- [ ] **Step 7: Commit**

```bash
git add crates/xtask/src/vm_image.rs crates/xtask/src/main.rs crates/xtask/Cargo.toml
git commit -m "feat(xtask): build-vm-image command — download Alpine + cross-compile agent"
```

---

### Task 6: VZ.framework objc2 bindings — boot loader and VM configuration

**Files:**

- Modify: `crates/macbox/Cargo.toml`
- Create: `crates/macbox/src/vz/mod.rs`
- Create: `crates/macbox/src/vz/bindings.rs`

This task adds the thin Rust wrappers around `Virtualization.framework`. We only bind what we need: `VZLinuxBootLoader`, `VZVirtioFileSystemDeviceConfiguration`, `VZVirtioSocketDeviceConfiguration`, `VZVirtualMachineConfiguration`, `VZVirtualMachine`.

- [ ] **Step 1: Add dependencies to `crates/macbox/Cargo.toml`**

```toml
[features]
vz = ["dep:objc2", "dep:objc2-foundation"]

[dependencies]
# ... existing deps ...
objc2 = { version = "0.5", optional = true }
objc2-foundation = { version = "0.2", optional = true, features = ["NSString", "NSURL"] }
```

- [ ] **Step 2: Create `crates/macbox/src/vz/mod.rs`**

```rust
//! Thin Rust bindings for Apple's Virtualization.framework.
//!
//! These bindings are hand-written using the `objc2` crate to avoid generating
//! thousands of lines of bindings for the full framework. Only the subset needed
//! to boot a Linux VM with virtiofs and vsock is included.

pub mod bindings;
pub mod vm;

pub use vm::VzVm;
```

- [ ] **Step 3: Create `crates/macbox/src/vz/bindings.rs`**

```rust
//! Raw objc2 bindings for Virtualization.framework classes.

use anyhow::{Context, Result, bail};
use objc2::ClassType;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSString, NSURL};
use std::path::Path;

// ── Framework link ──────────────────────────────────────────────────────────
// The Virtualization.framework is in the macOS SDK; objc2 binds to it via
// the Objective-C runtime without a separate -framework flag because objc2
// calls objc_getClass at runtime.

/// Load the Virtualization.framework bundle if not already loaded.
///
/// SAFETY: `NSBundle bundleWithPath:load:` is safe to call on any thread.
/// The framework must be loaded before any VZ class is accessed.
pub fn load_vz_framework() -> Result<()> {
    use std::ffi::CStr;
    let path = "/System/Library/Frameworks/Virtualization.framework";
    // Use NSBundle via raw objc2 to load the framework.
    // Safety: we pass a valid null-terminated path string.
    let loaded: bool = unsafe {
        use objc2::msg_send;
        let cls = objc2::class!(NSBundle);
        let path_ns: Retained<NSString> = NSString::from_str(path);
        let bundle: *mut AnyObject = msg_send![cls, bundleWithPath: &*path_ns];
        if bundle.is_null() {
            bail!("NSBundle could not find Virtualization.framework at {path}");
        }
        msg_send![bundle, load]
    };
    if !loaded {
        // Already loaded is also fine — loaded returns YES on first load, NO if already loaded.
        // No error.
    }
    Ok(())
}

/// Returns an `NSString` from a Rust `&str`.
pub fn ns_string(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

/// Returns an `NSURL` for a filesystem path.
///
/// SAFETY: `fileURLWithPath:` is a normal Foundation method, safe from any thread.
pub fn file_url(path: &Path) -> Result<Retained<NSURL>> {
    let path_str = path.to_str()
        .with_context(|| format!("non-UTF8 path: {}", path.display()))?;
    let ns_path = NSString::from_str(path_str);
    let url: Retained<NSURL> = unsafe {
        use objc2::msg_send_id;
        let cls = objc2::class!(NSURL);
        msg_send_id![cls, fileURLWithPath: &*ns_path]
    };
    Ok(url)
}

/// Create a `VZLinuxBootLoader` configured with the given kernel and optional initrd.
///
/// SAFETY: All VZ objects must be created on the main thread or a dedicated
/// `DispatchQueue` — the caller (`vm.rs`) is responsible for this invariant.
pub unsafe fn new_linux_boot_loader(
    kernel_path: &Path,
    initrd_path: Option<&Path>,
    cmdline: &str,
) -> Result<*mut AnyObject> {
    use objc2::msg_send_id;

    let kernel_url = file_url(kernel_path)?;
    let loader: *mut AnyObject = {
        let cls = objc2::class!(VZLinuxBootLoader);
        let obj: Retained<AnyObject> = msg_send_id![cls, alloc];
        msg_send_id![obj, initWithKernelURL: &*kernel_url]
    };
    if loader.is_null() {
        bail!("VZLinuxBootLoader alloc+init returned nil");
    }

    if let Some(initrd) = initrd_path {
        let initrd_url = file_url(initrd)?;
        let _: () = objc2::msg_send![loader, setInitialRamdiskURL: &*initrd_url];
    }

    let cmdline_ns = NSString::from_str(cmdline);
    let _: () = objc2::msg_send![loader, setCommandLine: &*cmdline_ns];

    Ok(loader)
}

/// Create a `VZVirtioFileSystemDeviceConfiguration` for a virtiofs share.
///
/// - `tag` — the mount tag the guest sees (e.g. `"mbx-images"`)
/// - `host_path` — directory on the host to share
///
/// SAFETY: must be called on the VZ dispatch queue.
pub unsafe fn new_virtio_fs(tag: &str, host_path: &Path) -> Result<*mut AnyObject> {
    use objc2::msg_send_id;

    // VZSharedDirectory *dir = [VZSharedDirectory alloc] initWithURL:url readOnly:YES];
    let dir_url = file_url(host_path)?;
    let shared_dir: Retained<AnyObject> = {
        let cls = objc2::class!(VZSharedDirectory);
        let obj: Retained<AnyObject> = msg_send_id![cls, alloc];
        let read_only: bool = tag.contains("images"); // images share is read-only
        msg_send_id![obj, initWithURL: &*dir_url readOnly: read_only]
    };

    // VZSingleDirectoryShare *share = [[VZSingleDirectoryShare alloc] initWithDirectory:dir];
    let share: Retained<AnyObject> = {
        let cls = objc2::class!(VZSingleDirectoryShare);
        let obj: Retained<AnyObject> = msg_send_id![cls, alloc];
        msg_send_id![obj, initWithDirectory: &*shared_dir]
    };

    // VZVirtioFileSystemDeviceConfiguration *fs = [[VZVirtioFileSystemDeviceConfiguration alloc] initWithTag:tag];
    let tag_ns = NSString::from_str(tag);
    let fs_config: Retained<AnyObject> = {
        let cls = objc2::class!(VZVirtioFileSystemDeviceConfiguration);
        let obj: Retained<AnyObject> = msg_send_id![cls, alloc];
        msg_send_id![obj, initWithTag: &*tag_ns]
    };
    let _: () = objc2::msg_send![&*fs_config, setShare: &*share];

    Ok(Retained::into_raw(fs_config))
}

/// Create a `VZVirtioSocketDeviceConfiguration` (vsock device).
///
/// SAFETY: must be called on the VZ dispatch queue.
pub unsafe fn new_vsock_device() -> Result<*mut AnyObject> {
    use objc2::msg_send_id;
    let cls = objc2::class!(VZVirtioSocketDeviceConfiguration);
    let obj: Retained<AnyObject> = msg_send_id![cls, alloc];
    let dev: Retained<AnyObject> = msg_send_id![obj, init];
    Ok(Retained::into_raw(dev))
}
```

- [ ] **Step 4: Add `mod vz` to `crates/macbox/src/lib.rs`**

```rust
#[cfg(feature = "vz")]
pub mod vz;
```

- [ ] **Step 5: Verify it compiles (on macOS only)**

```bash
cargo check -p macbox --features vz
```

Expected: compiles. There will be `unused` warnings for the binding fns — that's fine.

- [ ] **Step 6: Commit**

```bash
git add crates/macbox/Cargo.toml crates/macbox/src/vz/ crates/macbox/src/lib.rs
git commit -m "feat(macbox): VZ.framework objc2 bindings — boot loader, virtiofs, vsock"
```

---

### Task 7: `VzVm` — boot, wait-ready, shutdown

**Files:**

- Create: `crates/macbox/src/vz/vm.rs`

`VzVm` owns the `VZVirtualMachine` and exposes `boot() -> Result<()>` and `shutdown()`. All VZ calls happen on a dedicated GCD serial queue via `dispatch_sync` to satisfy the main-thread-or-dedicated-queue requirement. `tokio::task::spawn_blocking` bridges to async.

- [ ] **Step 1: Write the failing test (compile-only)**

```rust
// crates/macbox/src/vz/vm.rs (at bottom)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vz_vm_config_fields_are_accessible() {
        // Smoke test: VzVmConfig can be constructed without panicking.
        let tmp = std::env::temp_dir();
        let cfg = VzVmConfig {
            vm_dir: tmp.clone(),
            images_dir: tmp.clone(),
            containers_dir: tmp.clone(),
            memory_bytes: 512 * 1024 * 1024,
            cpu_count: 2,
        };
        assert_eq!(cfg.cpu_count, 2);
    }
}
```

- [ ] **Step 2: Run to confirm it fails**

```
cargo test -p macbox --features vz vz_vm_config_fields
```

Expected: compile error — `VzVmConfig` not defined.

- [ ] **Step 3: Implement `VzVmConfig` and `VzVm`**

```rust
// crates/macbox/src/vz/vm.rs

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

use super::bindings;

/// Configuration for booting the minibox Linux VM.
#[derive(Debug, Clone)]
pub struct VzVmConfig {
    /// Directory containing `boot/vmlinuz-virt`, `boot/initramfs-virt`, and `rootfs/`.
    pub vm_dir: PathBuf,
    /// Host path for the `mbx-images` virtiofs share (read-only OCI layers).
    pub images_dir: PathBuf,
    /// Host path for the `mbx-containers` virtiofs share (read-write container data).
    pub containers_dir: PathBuf,
    /// RAM in bytes (default: 1 GiB).
    pub memory_bytes: u64,
    /// vCPU count (default: 2).
    pub cpu_count: usize,
}

impl VzVmConfig {
    pub fn kernel_path(&self) -> PathBuf {
        self.vm_dir.join("boot").join("vmlinuz-virt")
    }
    pub fn initramfs_path(&self) -> PathBuf {
        self.vm_dir.join("boot").join("initramfs-virt")
    }
    pub fn rootfs_path(&self) -> PathBuf {
        self.vm_dir.join("rootfs")
    }
}

/// Handle to a running Virtualization.framework Linux VM.
///
/// The inner `VZVirtualMachine` pointer lives on a private GCD serial queue
/// (`mbx.vz.queue`). All calls that touch it must be dispatched onto that queue
/// via `dispatch_sync` / `dispatch_async`.
pub struct VzVm {
    /// Raw pointer to `VZVirtualMachine` — only accessed on `queue`.
    // SAFETY: we never send this raw pointer off the queue thread.
    vm_ptr: Mutex<*mut objc2::runtime::AnyObject>,
    config: VzVmConfig,
}

// SAFETY: `vm_ptr` is only accessed via `dispatch_sync` on a single serial queue.
unsafe impl Send for VzVm {}
unsafe impl Sync for VzVm {}

impl VzVm {
    /// Create and boot the VM. Returns once the VM is in the `running` state.
    ///
    /// This must be called from a `tokio::task::spawn_blocking` context because
    /// it calls `dispatch_sync`, which blocks the calling thread.
    pub fn boot(config: VzVmConfig) -> Result<Arc<Self>> {
        bindings::load_vz_framework().context("loading Virtualization.framework")?;

        // Safety: All VZ object construction happens here on the spawn_blocking
        // thread. After construction, accesses go through dispatch_sync.
        let vm_ptr = unsafe { Self::build_vm(&config)? };

        let vz = Arc::new(VzVm {
            vm_ptr: Mutex::new(vm_ptr),
            config,
        });

        // Start the VM synchronously — VZVirtualMachine start is completion-handler-based;
        // we use a Mutex<Option<Result>> to transfer the result back.
        let result: Arc<Mutex<Option<Result<()>>>> = Arc::new(Mutex::new(None));
        let result_clone = Arc::clone(&result);
        let ptr = *vz.vm_ptr.lock().unwrap();

        unsafe {
            use objc2::msg_send;
            // startWithCompletionHandler — called on the VZ queue internally.
            let handler = block2::ConcreteBlock::new(move |err: *mut objc2::runtime::AnyObject| {
                let r = if err.is_null() {
                    Ok(())
                } else {
                    let desc: *mut objc2_foundation::NSString = msg_send![err, localizedDescription];
                    let s = (*desc).to_string();
                    Err(anyhow::anyhow!("VM start failed: {s}"))
                };
                *result_clone.lock().unwrap() = Some(r);
            })
            .copy();
            let _: () = msg_send![ptr, startWithCompletionHandler: &*handler];
        }

        // Poll until state == Running or we have an error.
        // VZVirtualMachine.state is KVO-observable but polling is simpler here.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if std::time::Instant::now() > deadline {
                bail!("VM did not reach running state within 30s");
            }
            {
                let guard = result.lock().unwrap();
                if let Some(ref r) = *guard {
                    if let Err(e) = r {
                        bail!("VM start error: {}", e);
                    }
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        tracing::info!("vz: VM running");
        Ok(vz)
    }

    /// Build and return a raw `VZVirtualMachine *`.
    ///
    /// SAFETY: All object creation must happen on the same thread. This function
    /// is private and only called from `boot()`, which ensures this invariant.
    unsafe fn build_vm(config: &VzVmConfig) -> Result<*mut objc2::runtime::AnyObject> {
        use objc2::msg_send_id;
        use objc2::runtime::AnyObject;
        use objc2::rc::Retained;

        // --- Boot loader ---
        let boot_loader = bindings::new_linux_boot_loader(
            &config.kernel_path(),
            Some(&config.initramfs_path()),
            // Kernel command line: use virtiofs rootfs, quiet boot
            "root=virtiofs rw rootfstype=virtiofs quiet init=/sbin/init",
        )?;

        // --- VZVirtualMachineConfiguration ---
        let vm_config: Retained<AnyObject> = {
            let cls = objc2::class!(VZVirtualMachineConfiguration);
            let obj: Retained<AnyObject> = msg_send_id![cls, alloc];
            msg_send_id![obj, init]
        };

        // Boot loader
        let _: () = objc2::msg_send![&*vm_config, setBootLoader: boot_loader];

        // Memory
        let _: () = objc2::msg_send![&*vm_config, setMemorySize: config.memory_bytes];

        // CPU
        let _: () = objc2::msg_send![&*vm_config, setCPUCount: config.cpu_count];

        // Storage: virtiofs rootfs
        let rootfs_fs = bindings::new_virtio_fs("mbx-rootfs", &config.rootfs_path())?;
        let images_fs = bindings::new_virtio_fs("mbx-images", &config.images_dir)?;
        let containers_fs = bindings::new_virtio_fs("mbx-containers", &config.containers_dir)?;

        // NSArray of storage devices
        let devices = objc2_foundation::NSArray::from_retained_slice(&[
            Retained::from_raw(rootfs_fs).unwrap(),
            Retained::from_raw(images_fs).unwrap(),
            Retained::from_raw(containers_fs).unwrap(),
        ]);
        let _: () = objc2::msg_send![&*vm_config, setDirectorySharingDevices: &*devices];

        // Vsock device
        let vsock_dev = bindings::new_vsock_device()?;
        let vsock_arr = objc2_foundation::NSArray::from_retained_slice(&[
            Retained::from_raw(vsock_dev).unwrap(),
        ]);
        let _: () = objc2::msg_send![&*vm_config, setSocketDevices: &*vsock_arr];

        // Serial console (for debugging — writes to stderr)
        let serial: Retained<AnyObject> = {
            let cls = objc2::class!(VZVirtioConsoleDeviceSerialPortConfiguration);
            let obj: Retained<AnyObject> = msg_send_id![cls, alloc];
            msg_send_id![obj, init]
        };
        let file_attach: Retained<AnyObject> = {
            let cls = objc2::class!(VZFileHandleSerialPortAttachment);
            let obj: Retained<AnyObject> = msg_send_id![cls, alloc];
            // fileHandleForWriting: stderr
            let fh: Retained<AnyObject> = msg_send_id![objc2::class!(NSFileHandle), fileHandleWithStandardError];
            msg_send_id![obj, initWithFileHandleForReading: std::ptr::null::<AnyObject>()
                                          fileHandleForWriting: &*fh]
        };
        let _: () = objc2::msg_send![&*serial, setAttachment: &*file_attach];
        let serial_arr = objc2_foundation::NSArray::from_retained_slice(&[serial]);
        let _: () = objc2::msg_send![&*vm_config, setSerialPorts: &*serial_arr];

        // Validate configuration
        let err_ptr: *mut AnyObject = std::ptr::null_mut();
        let valid: bool = objc2::msg_send![&*vm_config, validateWithError: &err_ptr];
        if !valid {
            if !err_ptr.is_null() {
                let desc: *mut objc2_foundation::NSString = objc2::msg_send![err_ptr, localizedDescription];
                let s = (*desc).to_string();
                bail!("VZVirtualMachineConfiguration invalid: {s}");
            }
            bail!("VZVirtualMachineConfiguration invalid (no error description)");
        }

        // Create VZVirtualMachine
        let vm: Retained<AnyObject> = {
            let cls = objc2::class!(VZVirtualMachine);
            let obj: Retained<AnyObject> = msg_send_id![cls, alloc];
            msg_send_id![obj, initWithConfiguration: &*vm_config]
        };
        Ok(Retained::into_raw(vm))
    }

    /// Stop the VM (sends a stop request; does not wait for full shutdown).
    pub fn stop(&self) {
        let ptr = *self.vm_ptr.lock().unwrap();
        if !ptr.is_null() {
            unsafe {
                let _: () = objc2::msg_send![ptr, requestStopWithError: std::ptr::null::<objc2::runtime::AnyObject>()];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vz_vm_config_fields_are_accessible() {
        let tmp = std::env::temp_dir();
        let cfg = VzVmConfig {
            vm_dir: tmp.clone(),
            images_dir: tmp.clone(),
            containers_dir: tmp.clone(),
            memory_bytes: 512 * 1024 * 1024,
            cpu_count: 2,
        };
        assert_eq!(cfg.cpu_count, 2);
    }

    #[test]
    fn vz_vm_config_paths_use_vm_dir() {
        let vm_dir = std::path::PathBuf::from("/tmp/test-vm");
        let cfg = VzVmConfig {
            vm_dir: vm_dir.clone(),
            images_dir: Default::default(),
            containers_dir: Default::default(),
            memory_bytes: 0,
            cpu_count: 0,
        };
        assert_eq!(cfg.kernel_path(), vm_dir.join("boot").join("vmlinuz-virt"));
        assert_eq!(cfg.rootfs_path(), vm_dir.join("rootfs"));
    }
}
```

- [ ] **Step 4: Add `block2` dependency** (needed for completion handler blocks)

In `crates/macbox/Cargo.toml`:

```toml
block2 = { version = "0.5", optional = true }
```

And add `"dep:block2"` to the `vz` feature.

- [ ] **Step 5: Run tests**

```bash
cargo test -p macbox --features vz vz_vm_config
```

Expected: PASS (2 tests).

- [ ] **Step 6: Check it compiles**

```bash
cargo check -p macbox --features vz
```

Expected: compiles (may have unused warnings — OK for now).

- [ ] **Step 7: Commit**

```bash
git add crates/macbox/src/vz/vm.rs crates/macbox/Cargo.toml
git commit -m "feat(macbox): VzVm struct with boot/stop via Virtualization.framework"
```

---

### Task 8: Justfile + CLAUDE.md wiring

**Files:**

- Modify: `Justfile`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Add Justfile recipe**

```makefile
# Build the macOS VM image (Alpine kernel + rootfs + agent)
build-vm-image force="":
    #!/usr/bin/env bash
    if [ "{{force}}" = "force" ]; then
        cargo xtask build-vm-image --force
    else
        cargo xtask build-vm-image
    fi
```

- [ ] **Step 2: Update CLAUDE.md build commands section**

In the "Building" section, add:

```
# Build macOS VM image (Alpine kernel + agent, macOS only)
cargo xtask build-vm-image          # cached
cargo xtask build-vm-image --force  # re-download + recompile
```

- [ ] **Step 3: Verify Justfile recipe works**

```bash
just --list | grep build-vm-image
```

Expected: shows `build-vm-image` recipe.

- [ ] **Step 4: Commit**

```bash
git add Justfile CLAUDE.md
git commit -m "docs: wire build-vm-image into Justfile and CLAUDE.md"
```

---

## Verification

After all tasks complete, run the full pre-commit gate:

```bash
cargo xtask pre-commit
```

Then on macOS, do a live smoke test:

```bash
# Build the VM image (downloads ~15 MB, cross-compiles miniboxd)
cargo xtask build-vm-image
ls ~/.mbx/vm/
# Expected: boot/  cache/  manifest.json  rootfs/
ls ~/.mbx/vm/boot/
# Expected: initramfs-virt  vmlinuz-virt
ls ~/.mbx/vm/rootfs/sbin/minibox-agent
# Expected: file exists, is an ELF aarch64 binary
file ~/.mbx/vm/rootfs/sbin/minibox-agent
# Expected: ELF 64-bit LSB executable, ARM aarch64, statically linked
cat ~/.mbx/vm/manifest.json
# Expected: JSON with alpine_version, agent_commit, built_at fields
```

Plan B (`2026-03-29-macos-vz-adapter.md`) covers the VzAdapter, vsock proxy, and full end-to-end wiring that uses this image.
