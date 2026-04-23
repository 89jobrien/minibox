# QEMU Cross-Platform VM Runner Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `cargo xtask test-vm` and `cargo xtask build-vm-image` work on Linux x86_64
and Linux arm64 CI hosts by detecting the host platform at runtime and selecting the correct
QEMU binary, accelerator, Alpine arch, and musl cross-compile target.

**Architecture:** Add a `HostPlatform` enum to `xtask/src/vm_run.rs` detected via
`std::env::consts::{OS, ARCH}`. Thread it through `vm_run.rs` and `vm_image.rs`, replacing
every hardcoded `aarch64`/`hvf` reference. Phase C then extracts the QEMU lifecycle into a
`VmRunner` struct with a `VmHandle` to provide a seam for the future `QemuRuntime` adapter.

**Tech Stack:** Rust std only (`std::env::consts`, `std::process::Command`). No new
dependencies. `cargo xtask` (workspace dev tool). QEMU (external binary on host PATH).

---

## File Map

| File | Change |
|---|---|
| `crates/xtask/src/vm_run.rs` | Add `HostPlatform` enum + methods; update `run_vm_interactive` and `test_vm` to accept `&HostPlatform`; Phase C: refactor into `VmRunner` + `VmHandle` |
| `crates/xtask/src/vm_image.rs` | Replace `ALPINE_ARCH` constant usage with `HostPlatform::alpine_arch()`; update `build_and_install_agent` to accept `musl_target`; update `build_vm_image` to thread platform through |
| `crates/xtask/src/main.rs` | Detect `HostPlatform::detect()?` once in `run-vm`, `test-vm`, `build-vm-image` arms; pass to callsites |

---

## Phase A: Accelerator + Arch Detection

### Task 1: Add `HostPlatform` to `vm_run.rs` with unit tests

**Files:**
- Modify: `crates/xtask/src/vm_run.rs`

- [ ] **Step 1: Write the failing tests**

Add at the bottom of `crates/xtask/src/vm_run.rs`, inside a new `#[cfg(test)] mod tests` block:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_platform_macos_arm64() {
        let p = HostPlatform::from_parts("macos", "aarch64").unwrap();
        assert_eq!(p.qemu_binary(), "qemu-system-aarch64");
        assert_eq!(p.accel(), "hvf");
        assert_eq!(p.alpine_arch(), "aarch64");
        assert_eq!(p.musl_target(), "aarch64-unknown-linux-musl");
        assert_eq!(p.machine_type(), "virt");
    }

    #[test]
    fn host_platform_linux_x86_64() {
        let p = HostPlatform::from_parts("linux", "x86_64").unwrap();
        assert_eq!(p.qemu_binary(), "qemu-system-x86_64");
        assert_eq!(p.accel(), "kvm");
        assert_eq!(p.alpine_arch(), "x86_64");
        assert_eq!(p.musl_target(), "x86_64-unknown-linux-musl");
        assert_eq!(p.machine_type(), "virt");
    }

    #[test]
    fn host_platform_linux_arm64() {
        let p = HostPlatform::from_parts("linux", "aarch64").unwrap();
        assert_eq!(p.qemu_binary(), "qemu-system-aarch64");
        assert_eq!(p.accel(), "kvm");
        assert_eq!(p.alpine_arch(), "aarch64");
        assert_eq!(p.musl_target(), "aarch64-unknown-linux-musl");
        assert_eq!(p.machine_type(), "virt");
    }

    #[test]
    fn host_platform_unsupported_os_returns_err() {
        let result = HostPlatform::from_parts("windows", "x86_64");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("windows"), "error should mention OS: {msg}");
    }

    #[test]
    fn host_platform_unsupported_arch_returns_err() {
        let result = HostPlatform::from_parts("linux", "riscv64");
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p xtask 2>&1 | tail -20
```

Expected: compile error — `HostPlatform` not defined yet.

- [ ] **Step 3: Add `HostPlatform` enum and impl**

Add this block at the top of `crates/xtask/src/vm_run.rs`, before the existing
`const QEMU_BASE_ARGS` line:

```rust
/// Host platform detected at runtime. Determines QEMU binary, accelerator,
/// Alpine arch, and musl cross-compile target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostPlatform {
    MacOsArm64,
    LinuxX86_64,
    LinuxArm64,
}

impl HostPlatform {
    /// Detect from `std::env::consts::{OS, ARCH}`.
    pub fn detect() -> anyhow::Result<Self> {
        Self::from_parts(std::env::consts::OS, std::env::consts::ARCH)
    }

    /// Construct from explicit OS/arch strings. Used by tests.
    pub fn from_parts(os: &str, arch: &str) -> anyhow::Result<Self> {
        match (os, arch) {
            ("macos", "aarch64") => Ok(Self::MacOsArm64),
            ("linux", "x86_64") => Ok(Self::LinuxX86_64),
            ("linux", "aarch64") => Ok(Self::LinuxArm64),
            _ => anyhow::bail!(
                "unsupported host: os={os} arch={arch}\n  \
                 QEMU VM runner requires macOS arm64 (hvf) or Linux x86_64/arm64 (kvm)."
            ),
        }
    }

    pub fn qemu_binary(&self) -> &'static str {
        match self {
            Self::MacOsArm64 | Self::LinuxArm64 => "qemu-system-aarch64",
            Self::LinuxX86_64 => "qemu-system-x86_64",
        }
    }

    pub fn accel(&self) -> &'static str {
        match self {
            Self::MacOsArm64 => "hvf",
            Self::LinuxX86_64 | Self::LinuxArm64 => "kvm",
        }
    }

    pub fn alpine_arch(&self) -> &'static str {
        match self {
            Self::MacOsArm64 | Self::LinuxArm64 => "aarch64",
            Self::LinuxX86_64 => "x86_64",
        }
    }

    pub fn musl_target(&self) -> &'static str {
        match self {
            Self::MacOsArm64 | Self::LinuxArm64 => "aarch64-unknown-linux-musl",
            Self::LinuxX86_64 => "x86_64-unknown-linux-musl",
        }
    }

    /// QEMU machine type. Currently `virt` for all platforms.
    /// Revisit when wiring as a runtime adapter (Phase B).
    pub fn machine_type(&self) -> &'static str {
        "virt"
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p xtask 2>&1 | tail -20
```

Expected: all `host_platform_*` tests pass. Existing tests (`manifest_roundtrip`, etc.) still pass.

- [ ] **Step 5: Commit**

```bash
git -C /Users/joe/dev/minibox add crates/xtask/src/vm_run.rs
git -C /Users/joe/dev/minibox commit -m "feat(xtask): add HostPlatform enum with detect() and unit tests"
```

---

### Task 2: Thread `HostPlatform` through `run_vm_interactive` and `test_vm`

**Files:**
- Modify: `crates/xtask/src/vm_run.rs`

- [ ] **Step 1: Replace `const QEMU_BASE_ARGS` and update function signatures**

Remove the existing `const QEMU_BASE_ARGS` line:
```rust
const QEMU_BASE_ARGS: &[&str] = &[
    "-M", "virt", "-cpu", "host", "-accel", "hvf", "-m", "2048", "-smp", "4", "-kernel",
];
```

Update `run_vm_interactive` signature and body. Replace the full function with:

```rust
/// Boot the VM in interactive shell mode.  Blocks until QEMU exits.
/// Exit QEMU with `Ctrl-A X`.
pub fn run_vm_interactive(vm_dir: &Path, platform: &HostPlatform) -> Result<()> {
    let kernel = vm_dir.join("boot").join("vmlinuz-virt");
    let initrd = vm_dir.join("minibox-initramfs.img");

    if !kernel.exists() {
        bail!(
            "kernel not found at {}; run `cargo xtask build-vm-image` first",
            kernel.display()
        );
    }
    if !initrd.exists() {
        bail!(
            "initramfs not found at {}; run `cargo xtask build-vm-image` first",
            initrd.display()
        );
    }

    println!("Booting minibox VM — interactive shell");
    println!("  Exit: Ctrl-A X");
    println!();

    let status = std::process::Command::new(platform.qemu_binary())
        .args([
            "-M", platform.machine_type(),
            "-cpu", "host",
            "-accel", platform.accel(),
            "-m", "2048",
            "-smp", "4",
            "-kernel",
        ])
        .arg(&kernel)
        .arg("-initrd")
        .arg(&initrd)
        .args([
            "-append",
            "rdinit=/sbin/init console=ttyAMA0,115200 minibox.mode=shell",
            "-nographic",
            "-no-reboot",
        ])
        .status()
        .with_context(|| format!("spawning {} (is QEMU installed?)", platform.qemu_binary()))?;

    if !status.success() {
        bail!("QEMU exited with status {}", status);
    }
    Ok(())
}
```

Update `test_vm` — change the signature line and replace the two hardcoded strings inside it:

```rust
pub fn test_vm(vm_dir: &Path, cargo_target: &Path, platform: &HostPlatform) -> Result<()> {
```

Inside `test_vm`, replace:
```rust
    let target = "aarch64-unknown-linux-musl";
```
with:
```rust
    let target = platform.musl_target();
```

Replace the `Command::new("qemu-system-aarch64")` spawn block with:
```rust
    let mut child = std::process::Command::new(platform.qemu_binary())
        .args([
            "-M", platform.machine_type(),
            "-cpu", "host",
            "-accel", platform.accel(),
            "-m", "2048",
            "-smp", "4",
            "-kernel",
        ])
        .arg(&kernel)
        .arg("-initrd")
        .arg(&initrd)
        .args([
            "-append",
            "rdinit=/sbin/init console=ttyAMA0,115200 minibox.mode=test",
            "-serial",
        ])
        .arg(&serial_arg)
        .args(["-display", "none", "-monitor", "none", "-no-reboot"])
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawning {}", platform.qemu_binary()))?;
```

- [ ] **Step 2: Fix compile errors**

```bash
cargo check -p xtask 2>&1
```

Fix any remaining compile errors (callers in `main.rs` will be updated in Task 3).
If `main.rs` fails to compile, add `todo!()` stubs temporarily — we fix it properly in Task 3.

- [ ] **Step 3: Run tests**

```bash
cargo test -p xtask 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git -C /Users/joe/dev/minibox add crates/xtask/src/vm_run.rs
git -C /Users/joe/dev/minibox commit -m "feat(xtask): thread HostPlatform through run_vm_interactive and test_vm"
```

---

### Task 3: Update `vm_image.rs` to use `HostPlatform`

**Files:**
- Modify: `crates/xtask/src/vm_image.rs`

- [ ] **Step 1: Write a failing test for platform-aware asset URLs**

Add to the `#[cfg(test)] mod tests` block at the bottom of `vm_image.rs`:

```rust
    #[test]
    fn alpine_urls_x86_64() {
        // Verify x86_64 arch produces correct URLs — regression guard for
        // hardcoded-aarch64 bug.
        let urls = AlpineAssets::for_version("3.21.3", "x86_64");
        assert!(urls.kernel.contains("x86_64"), "kernel URL should contain arch");
        assert!(
            urls.minirootfs.contains("alpine-minirootfs-3.21.3-x86_64.tar.gz"),
            "minirootfs URL should contain x86_64: {}",
            urls.minirootfs
        );
    }

    #[test]
    fn build_vm_image_uses_platform_arch() {
        // Verify the tarball cache path uses the platform arch, not a hardcoded value.
        // We test this structurally: build_vm_image with a pre-cached x86_64 tarball
        // and confirm it doesn't look for an aarch64 tarball.
        let tmp = tempfile::tempdir().unwrap();
        let vm_dir = tmp.path().join("vm");

        let cache_dir = vm_dir.join("cache");
        let rootfs_dir = vm_dir.join("rootfs");
        let boot_dir = vm_dir.join("boot");
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::create_dir_all(&rootfs_dir).unwrap();
        std::fs::create_dir_all(&boot_dir).unwrap();

        std::fs::write(boot_dir.join("vmlinuz-virt"), b"fake kernel").unwrap();
        std::fs::write(boot_dir.join("initramfs-virt"), b"fake initrd").unwrap();

        // x86_64 tarball cache path
        let tarball = cache_dir.join(format!(
            "alpine-minirootfs-{ALPINE_VERSION}-x86_64.tar.gz"
        ));
        std::fs::write(&tarball, b"fake tarball").unwrap();
        std::fs::create_dir_all(rootfs_dir.join("bin")).unwrap();
        std::fs::create_dir_all(rootfs_dir.join("sbin")).unwrap();
        std::fs::write(
            rootfs_dir.join("sbin").join("minibox-agent"),
            b"fake agent",
        )
        .unwrap();

        let platform = HostPlatform::LinuxX86_64;
        let result = build_vm_image_with_platform(&vm_dir, false, &platform);
        assert!(result.is_ok(), "build_vm_image_with_platform failed: {:?}", result);
        assert!(vm_dir.join("manifest.json").exists());
    }
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test -p xtask 2>&1 | tail -20
```

Expected: `build_vm_image_uses_platform_arch` fails — `build_vm_image_with_platform` not defined yet.

- [ ] **Step 3: Add `build_vm_image_with_platform` alongside existing `build_vm_image`**

In `vm_image.rs`, add this import at the top of the file (after existing imports):

```rust
use crate::vm_run::HostPlatform;
```

Add a new public function `build_vm_image_with_platform` that accepts an explicit platform.
Keep the existing `build_vm_image` as a thin wrapper that calls it:

```rust
/// Build or refresh the VM image directory using an explicit platform.
/// Downloads Alpine assets, extracts rootfs, cross-compiles agent, writes manifest.
pub fn build_vm_image_with_platform(
    vm_dir: &Path,
    force: bool,
    platform: &HostPlatform,
) -> Result<()> {
    println!("Building VM image in {}", vm_dir.display());

    let cache_dir = vm_dir.join("cache");
    let rootfs_dir = vm_dir.join("rootfs");
    let boot_dir = vm_dir.join("boot");
    let manifest_path = vm_dir.join("manifest.json");

    for dir in &[&cache_dir, &rootfs_dir, &boot_dir] {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }

    let arch = platform.alpine_arch();
    let assets = AlpineAssets::for_version(ALPINE_VERSION, arch);

    // 1. Download kernel
    let kernel_dest = boot_dir.join("vmlinuz-virt");
    download_file(&assets.kernel, &kernel_dest, force)?;

    // 2. Download initramfs
    let initramfs_dest = boot_dir.join("initramfs-virt");
    download_file(&assets.initramfs, &initramfs_dest, force)?;

    // 3. Download minirootfs tarball
    let tarball_dest = cache_dir.join(format!(
        "alpine-minirootfs-{ALPINE_VERSION}-{arch}.tar.gz"
    ));
    download_file(&assets.minirootfs, &tarball_dest, force)?;

    // 4. Extract rootfs
    extract_rootfs_if_needed(&tarball_dest, &rootfs_dir, force)?;

    // 4b. Apply user overlay (~/.minibox/vm/overlay/ → rootfs)
    install_overlay(&rootfs_dir, vm_dir)?;

    // 5. Cross-compile and install agent
    let rustc_ver = build_and_install_agent_for_target(&rootfs_dir, force, platform.musl_target())?;

    // 6. Install init files for PID-1 bootstrap
    install_init_files(&rootfs_dir)?;

    // 7. Build initramfs
    let our_initramfs = vm_dir.join("minibox-initramfs.img");
    create_initramfs(&rootfs_dir, &our_initramfs, force)?;

    // 8. Get current git commit hash for manifest
    let commit = {
        let out = std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .context("git rev-parse")?;
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

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
    println!("  initramfs {}", our_initramfs.display());
    println!("  rootfs    {}", rootfs_dir.display());
    Ok(())
}

/// Build or refresh the VM image directory, detecting platform automatically.
pub fn build_vm_image(vm_dir: &Path, force: bool) -> Result<()> {
    let platform = HostPlatform::detect()?;
    build_vm_image_with_platform(vm_dir, force, &platform)
}
```

Also add `build_and_install_agent_for_target` — a generalised version of
`build_and_install_agent` that takes an explicit `target` string. Keep the old function
as a wrapper:

```rust
/// Cross-compile miniboxd for the given musl target and copy into rootfs.
/// Skips if agent already exists at dest and `force` is false.
/// Returns the rustc version string (for the manifest).
pub fn build_and_install_agent_for_target(
    rootfs_dir: &Path,
    force: bool,
    target: &str,
) -> Result<String> {
    let dest = agent_dest_path(rootfs_dir);

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
        .args([
            "zigbuild",
            "--release",
            "--target",
            target,
            "-p",
            "miniboxd",
        ])
        .status()
        .context("cargo zigbuild for agent (is cargo-zigbuild installed?)")?;
    if !status.success() {
        anyhow::bail!("cargo zigbuild failed for miniboxd/{target}");
    }

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

/// Cross-compile miniboxd for aarch64-unknown-linux-musl and copy into rootfs.
/// Deprecated: use `build_and_install_agent_for_target` directly.
pub fn build_and_install_agent(rootfs_dir: &Path, force: bool) -> Result<String> {
    build_and_install_agent_for_target(rootfs_dir, force, "aarch64-unknown-linux-musl")
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p xtask 2>&1 | tail -30
```

Expected: all tests pass including `build_vm_image_uses_platform_arch` and `alpine_urls_x86_64`.

- [ ] **Step 5: Commit**

```bash
git -C /Users/joe/dev/minibox add crates/xtask/src/vm_image.rs
git -C /Users/joe/dev/minibox commit -m "feat(xtask): platform-aware build_vm_image_with_platform and build_and_install_agent_for_target"
```

---

### Task 4: Update `main.rs` callsites

**Files:**
- Modify: `crates/xtask/src/main.rs`

- [ ] **Step 1: Update the three xtask arms**

In `main.rs`, locate the `Some("build-vm-image")` arm. It already calls `build_vm_image`
which now auto-detects — no change needed there.

Locate the `Some("run-vm")` arm and update it:

```rust
        Some("run-vm") => {
            let vm_dir = vm_image::default_vm_dir();
            let platform = vm_run::HostPlatform::detect()?;
            vm_run::run_vm_interactive(&vm_dir, &platform)
        }
```

Locate the `Some("test-vm")` arm and update it:

```rust
        Some("test-vm") => {
            let vm_dir = vm_image::default_vm_dir();
            let cargo_target = env::var("CARGO_TARGET_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::Path::new("target").to_path_buf());
            let platform = vm_run::HostPlatform::detect()?;
            vm_run::test_vm(&vm_dir, &cargo_target, &platform)
        }
```

- [ ] **Step 2: Check it compiles**

```bash
cargo check -p xtask 2>&1
```

Expected: no errors.

- [ ] **Step 3: Run all xtask tests**

```bash
cargo test -p xtask 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git -C /Users/joe/dev/minibox add crates/xtask/src/main.rs
git -C /Users/joe/dev/minibox commit -m "feat(xtask): detect HostPlatform in run-vm and test-vm xtask arms"
```

---

### Task 5: Phase A acceptance — smoke test on macOS

**Files:** none (validation only)

- [ ] **Step 1: Run the full xtask test suite**

```bash
cargo test -p xtask 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 2: Run xtask pre-commit gate**

```bash
cargo xtask pre-commit 2>&1 | tail -20
```

Expected: fmt + clippy + build all pass.

- [ ] **Step 3: Verify detect() returns MacOsArm64 on this machine**

Add a temporary `println!` to `run-vm` arm, run it without a VM dir to see the error, confirm
platform detection string appears. Or just verify via test:

```bash
cargo test -p xtask host_platform 2>&1
```

Expected: all four `host_platform_*` tests pass.

- [ ] **Step 4: Tag Phase A complete in commit**

```bash
git -C /Users/joe/dev/minibox commit --allow-empty -m "chore(xtask): phase-a complete — qemu cross-platform detection"
```

---

## Phase C: `VmRunner` + `VmHandle` Refactor

### Task 6: Extract `VmHandle` from `test_vm`

**Files:**
- Modify: `crates/xtask/src/vm_run.rs`

- [ ] **Step 1: Write failing tests for `VmHandle`**

Add to the `#[cfg(test)] mod tests` block in `vm_run.rs`:

```rust
    #[test]
    fn vm_handle_serial_sock_path_is_absolute() {
        // VmHandle should store an absolute socket path derived from PID.
        let pid = std::process::id();
        let sock = format!("/tmp/minibox-vm-serial-{pid}.sock");
        // Just verify the format we expect — VmHandle is constructed by VmRunner::spawn_vm
        assert!(sock.starts_with("/tmp/minibox-vm-serial-"));
        assert!(sock.ends_with(".sock"));
    }
```

- [ ] **Step 2: Run to confirm test passes (it's structural, just format validation)**

```bash
cargo test -p xtask vm_handle 2>&1
```

- [ ] **Step 3: Define `VmHandle` struct**

Add after the `HostPlatform` impl block in `vm_run.rs`:

```rust
/// Handle to a running QEMU VM process. Owns the child process and serial socket path.
/// Created by `VmRunner::spawn_vm`. Drop kills the process and cleans up the socket.
pub struct VmHandle {
    pub child: std::process::Child,
    pub serial_sock: std::path::PathBuf,
}

impl VmHandle {
    /// Connect to the VM serial console. Retries for up to 10 seconds.
    pub fn connect_serial(&self) -> Result<std::os::unix::net::UnixStream> {
        let max_attempts = 50u32;
        for attempt in 0..max_attempts {
            match std::os::unix::net::UnixStream::connect(&self.serial_sock) {
                Ok(s) => return Ok(s),
                Err(_) if attempt + 1 < max_attempts => {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                Err(e) => {
                    anyhow::bail!(
                        "could not connect to VM serial socket after {}s: {e}",
                        max_attempts / 5
                    );
                }
            }
        }
        unreachable!()
    }

    /// Wait for the QEMU process to exit.
    pub fn wait(mut self) -> Result<std::process::ExitStatus> {
        let status = self.child.wait().context("waiting for QEMU child")?;
        let _ = std::fs::remove_file(&self.serial_sock);
        Ok(status)
    }

    /// Kill the QEMU process immediately.
    pub fn kill(mut self) -> Result<()> {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.serial_sock);
        Ok(())
    }
}
```

- [ ] **Step 4: Verify it compiles**

```bash
cargo check -p xtask 2>&1
```

- [ ] **Step 5: Commit**

```bash
git -C /Users/joe/dev/minibox add crates/xtask/src/vm_run.rs
git -C /Users/joe/dev/minibox commit -m "feat(xtask): add VmHandle struct with connect_serial/wait/kill"
```

---

### Task 7: Extract `VmRunner` struct

**Files:**
- Modify: `crates/xtask/src/vm_run.rs`

- [ ] **Step 1: Write failing tests for `VmRunner`**

Add to the `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn vm_runner_new_stores_fields() {
        use std::path::PathBuf;
        let platform = HostPlatform::LinuxX86_64;
        let vm_dir = PathBuf::from("/tmp/test-vm");
        let cargo_target = PathBuf::from("/tmp/target");
        let runner = VmRunner::new(platform.clone(), vm_dir.clone(), cargo_target.clone());
        assert_eq!(runner.platform, platform);
        assert_eq!(runner.vm_dir, vm_dir);
        assert_eq!(runner.cargo_target, cargo_target);
    }

    #[test]
    fn vm_runner_kernel_path() {
        use std::path::PathBuf;
        let runner = VmRunner::new(
            HostPlatform::MacOsArm64,
            PathBuf::from("/tmp/vm"),
            PathBuf::from("/tmp/target"),
        );
        assert_eq!(
            runner.kernel_path(),
            PathBuf::from("/tmp/vm/boot/vmlinuz-virt")
        );
    }
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test -p xtask vm_runner 2>&1
```

Expected: compile error — `VmRunner` not defined.

- [ ] **Step 3: Define `VmRunner` struct**

Add after the `VmHandle` impl block in `vm_run.rs`:

```rust
/// Orchestrates QEMU VM lifecycle for the xtask test harness.
/// Wraps platform detection, VM directory layout, and QEMU spawn/connect.
/// `spawn_vm` is the seam for the future Phase B `QemuRuntime` adapter.
pub struct VmRunner {
    pub platform: HostPlatform,
    pub vm_dir: std::path::PathBuf,
    pub cargo_target: std::path::PathBuf,
}

impl VmRunner {
    pub fn new(
        platform: HostPlatform,
        vm_dir: std::path::PathBuf,
        cargo_target: std::path::PathBuf,
    ) -> Self {
        Self { platform, vm_dir, cargo_target }
    }

    pub fn kernel_path(&self) -> std::path::PathBuf {
        self.vm_dir.join("boot").join("vmlinuz-virt")
    }

    /// Spawn a QEMU VM with the given kernel command-line append string.
    /// Returns a `VmHandle` that owns the child process and serial socket.
    pub fn spawn_vm(&self, kernel_cmdline: &str) -> Result<VmHandle> {
        let kernel = self.kernel_path();
        if !kernel.exists() {
            bail!(
                "kernel not found at {}; run `cargo xtask build-vm-image` first",
                kernel.display()
            );
        }

        let pid = std::process::id();
        let sock_path = format!("/tmp/minibox-vm-serial-{pid}.sock");
        let serial_arg = format!("unix:{sock_path},server,nowait");

        let child = std::process::Command::new(self.platform.qemu_binary())
            .args([
                "-M", self.platform.machine_type(),
                "-cpu", "host",
                "-accel", self.platform.accel(),
                "-m", "2048",
                "-smp", "4",
                "-kernel",
            ])
            .arg(&kernel)
            .args(["-append"])
            .arg(kernel_cmdline)
            .args(["-serial"])
            .arg(&serial_arg)
            .args(["-display", "none", "-monitor", "none", "-no-reboot"])
            .stdin(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("spawning {}", self.platform.qemu_binary()))?;

        Ok(VmHandle {
            child,
            serial_sock: std::path::PathBuf::from(sock_path),
        })
    }

    /// Run tests inside the VM. Stages binaries, spawns VM, streams serial output.
    pub fn run_tests(&self, suites: &[&str]) -> Result<()> {
        // Stage test binaries
        let target = self.platform.musl_target();
        let deps_dir = self.cargo_target
            .join(target)
            .join("debug")
            .join("deps");
        let tests_dir = self.vm_dir.join("rootfs").join("tests");
        std::fs::create_dir_all(&tests_dir).context("creating rootfs/tests")?;

        let mut copied = 0usize;
        for suite in suites {
            if let Ok(entries) = std::fs::read_dir(&deps_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if !name_str.starts_with(suite) || name_str.contains('.') {
                        continue;
                    }
                    let meta = entry.metadata().context("reading entry metadata")?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if meta.permissions().mode() & 0o111 == 0 {
                            continue;
                        }
                    }
                    #[cfg(not(unix))]
                    if !meta.is_file() {
                        continue;
                    }
                    let dest = tests_dir.join(&*name_str);
                    std::fs::copy(entry.path(), &dest)
                        .with_context(|| format!("copying {name_str} to rootfs/tests"))?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(
                            &dest,
                            std::fs::Permissions::from_mode(0o755),
                        )
                        .context("chmod test binary")?;
                    }
                    println!("  copied  tests/{name_str}");
                    copied += 1;
                    break;
                }
            }
        }

        let bin_dir = self.cargo_target.join(target).join("debug");
        for bin_name in &["miniboxd", "minibox"] {
            let src = bin_dir.join(bin_name);
            if src.exists() {
                let dest = tests_dir.join(bin_name);
                std::fs::copy(&src, &dest)
                    .with_context(|| format!("copying {bin_name}"))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(
                        &dest,
                        std::fs::Permissions::from_mode(0o755),
                    )
                    .context("chmod binary")?;
                }
                copied += 1;
            }
        }
        println!("  staged  {copied} binaries into rootfs/tests/");

        crate::vm_image::install_init_files(&self.vm_dir.join("rootfs"))?;
        let initrd = self.vm_dir.join("minibox-initramfs-test.img");
        crate::vm_image::create_initramfs(&self.vm_dir.join("rootfs"), &initrd, true)?;

        println!("Starting QEMU VM for tests...");
        let handle = self.spawn_vm(
            "rdinit=/sbin/init console=ttyAMA0,115200 minibox.mode=test",
        )?;

        let stream = handle.connect_serial()?;
        let reader = std::io::BufReader::new(stream);
        let mut final_rc: Option<i32> = None;

        use std::io::BufRead;
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    println!("[vm] {l}");
                    if let Some(rest) = l.strip_prefix("MINIBOX_TESTS_DONE rc=") {
                        final_rc = rest.trim().parse::<i32>().ok();
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("[vm] read error: {e}");
                    break;
                }
            }
        }

        handle.wait()?;

        match final_rc {
            Some(0) => {
                println!("All VM tests passed.");
                Ok(())
            }
            Some(n) => bail!("VM tests failed (rc={n})"),
            None => bail!(
                "VM tests did not produce a MINIBOX_TESTS_DONE sentinel — check VM output"
            ),
        }
    }

    /// Run the VM in interactive shell mode. Blocks until QEMU exits.
    pub fn run_interactive(&self) -> Result<()> {
        let kernel = self.kernel_path();
        if !kernel.exists() {
            bail!(
                "kernel not found at {}; run `cargo xtask build-vm-image` first",
                kernel.display()
            );
        }
        let initrd = self.vm_dir.join("minibox-initramfs.img");
        if !initrd.exists() {
            bail!(
                "initramfs not found at {}; run `cargo xtask build-vm-image` first",
                initrd.display()
            );
        }

        println!("Booting minibox VM — interactive shell");
        println!("  Exit: Ctrl-A X");
        println!();

        let status = std::process::Command::new(self.platform.qemu_binary())
            .args([
                "-M", self.platform.machine_type(),
                "-cpu", "host",
                "-accel", self.platform.accel(),
                "-m", "2048",
                "-smp", "4",
                "-kernel",
            ])
            .arg(&kernel)
            .arg("-initrd")
            .arg(&initrd)
            .args([
                "-append",
                "rdinit=/sbin/init console=ttyAMA0,115200 minibox.mode=shell",
                "-nographic",
                "-no-reboot",
            ])
            .status()
            .with_context(|| {
                format!("spawning {} (is QEMU installed?)", self.platform.qemu_binary())
            })?;

        if !status.success() {
            bail!("QEMU exited with status {}", status);
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p xtask vm_runner 2>&1
```

Expected: `vm_runner_new_stores_fields` and `vm_runner_kernel_path` pass.

- [ ] **Step 5: Commit**

```bash
git -C /Users/joe/dev/minibox add crates/xtask/src/vm_run.rs
git -C /Users/joe/dev/minibox commit -m "feat(xtask): add VmRunner struct with spawn_vm/run_tests/run_interactive"
```

---

### Task 8: Replace free functions with `VmRunner` in `main.rs`, keep old fns as thin wrappers

**Files:**
- Modify: `crates/xtask/src/vm_run.rs`
- Modify: `crates/xtask/src/main.rs`

- [ ] **Step 1: Update free functions to delegate to `VmRunner`**

Replace the bodies of `run_vm_interactive` and `test_vm` in `vm_run.rs` with thin
delegating wrappers. This preserves the public API while the implementation lives in
`VmRunner`:

```rust
/// Boot the VM in interactive shell mode. Thin wrapper over `VmRunner::run_interactive`.
pub fn run_vm_interactive(vm_dir: &Path, platform: &HostPlatform) -> Result<()> {
    let runner = VmRunner::new(
        platform.clone(),
        vm_dir.to_path_buf(),
        std::path::PathBuf::from("target"), // not used for interactive
    );
    runner.run_interactive()
}

/// Cross-compile test binaries then run them inside the VM.
/// Thin wrapper over `VmRunner::run_tests`.
pub fn test_vm(vm_dir: &Path, cargo_target: &Path, platform: &HostPlatform) -> Result<()> {
    let runner = VmRunner::new(
        platform.clone(),
        vm_dir.to_path_buf(),
        cargo_target.to_path_buf(),
    );
    let suites = &[
        "cgroup_tests",
        "e2e_tests",
        "integration_tests",
        "sandbox_tests",
    ];

    // Build test binaries first (cargo zigbuild step from original test_vm)
    let target = platform.musl_target();
    println!("Building test binaries for {target}...");
    let build_status = std::process::Command::new("cargo")
        .args(["zigbuild", "--tests", "-p", "miniboxd", "--target", target])
        .status()
        .context("cargo zigbuild --tests (is cargo-zigbuild installed?)")?;
    if !build_status.success() {
        bail!("cargo zigbuild --tests failed");
    }
    let bin_status = std::process::Command::new("cargo")
        .args(["zigbuild", "-p", "miniboxd", "-p", "minibox-cli", "--target", target])
        .status()
        .context("cargo zigbuild for miniboxd + minibox-cli")?;
    if !bin_status.success() {
        bail!("cargo zigbuild for binaries failed");
    }

    runner.run_tests(suites)
}
```

- [ ] **Step 2: Check it compiles**

```bash
cargo check -p xtask 2>&1
```

- [ ] **Step 3: Run all tests**

```bash
cargo test -p xtask 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 4: Run pre-commit gate**

```bash
cargo xtask pre-commit 2>&1 | tail -20
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
git -C /Users/joe/dev/minibox add crates/xtask/src/vm_run.rs crates/xtask/src/main.rs
git -C /Users/joe/dev/minibox commit -m "refactor(xtask): delegate free functions to VmRunner; VmRunner is Phase B seam"
```

---

### Task 9: Final cleanup and branch push

- [ ] **Step 1: Run full test suite**

```bash
cargo test -p xtask 2>&1 | tail -30
```

Expected: all pass.

- [ ] **Step 2: Run pre-commit gate**

```bash
cargo xtask pre-commit 2>&1 | tail -20
```

Expected: pass.

- [ ] **Step 3: Push branch**

```bash
git -C /Users/joe/dev/minibox push -u origin feat/qemu-cross-platform
```

- [ ] **Step 4: Update HANDOFF**

Note in HANDOFF that `feat/qemu-cross-platform` is complete through Phase C.
Phase B (`QemuRuntime` adapter, `MINIBOX_ADAPTER=qemu`) is a separate branch/spec.
The `VmRunner::spawn_vm` method is the Phase B seam.
