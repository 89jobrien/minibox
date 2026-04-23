//! vm_run — boot the minibox Alpine VM under QEMU with HVF acceleration.
//!
//! Two entry points:
//!   run_vm_interactive   interactive shell on serial console (blocks)
//!   test_vm              build musl test binaries + run in VM, stream results

use anyhow::{Context, Result, bail};
use std::{
    io::{BufRead, BufReader},
    os::unix::net::UnixStream,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

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

    let status = Command::new(platform.qemu_binary())
        .args([
            "-M",
            platform.machine_type(),
            "-cpu",
            "host",
            "-accel",
            platform.accel(),
            "-m",
            "2048",
            "-smp",
            "4",
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

/// Cross-compile test binaries then run them inside the VM, streaming output.
/// Exits with an error if any test suite fails.
pub fn test_vm(vm_dir: &Path, cargo_target: &Path, platform: &HostPlatform) -> Result<()> {
    let kernel = vm_dir.join("boot").join("vmlinuz-virt");
    let rootfs_dir = vm_dir.join("rootfs");

    if !rootfs_dir.exists() {
        bail!(
            "rootfs not found at {}; run `cargo xtask build-vm-image` first",
            rootfs_dir.display()
        );
    }

    // 1. Build test binaries for the target platform
    let target = platform.musl_target();
    println!("Building test binaries for {target}...");
    let build_status = Command::new("cargo")
        .args(["zigbuild", "--tests", "-p", "miniboxd", "--target", target])
        .status()
        .context("cargo zigbuild --tests (is cargo-zigbuild installed?)")?;
    if !build_status.success() {
        bail!("cargo zigbuild --tests failed");
    }

    // Also build miniboxd + minibox CLI binaries for MINIBOX_TEST_BIN_DIR
    let bin_status = Command::new("cargo")
        .args([
            "zigbuild",
            "-p",
            "miniboxd",
            "-p",
            "minibox-cli",
            "--target",
            target,
        ])
        .status()
        .context("cargo zigbuild for miniboxd + minibox-cli")?;
    if !bin_status.success() {
        bail!("cargo zigbuild for binaries failed");
    }

    // 2. Resolve absolute cargo target dir
    let cargo_target_abs = if cargo_target.is_absolute() {
        cargo_target.to_path_buf()
    } else {
        std::env::current_dir()
            .context("getting cwd")?
            .join(cargo_target)
    };
    let deps_dir = cargo_target_abs.join(target).join("debug").join("deps");

    // 3. Copy test binaries into rootfs/tests/ and rebuild initramfs
    let tests_dir = rootfs_dir.join("tests");
    std::fs::create_dir_all(&tests_dir).context("creating rootfs/tests")?;

    // Copy each test binary (executable files matching suite-* in deps/)
    let suites = [
        "cgroup_tests",
        "e2e_tests",
        "integration_tests",
        "sandbox_tests",
    ];
    let mut copied = 0usize;
    for suite in &suites {
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
                        continue; // skip non-executable
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
                    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
                        .context("chmod test binary")?;
                }
                println!("  copied  tests/{name_str}");
                copied += 1;
                break; // take the first match per suite
            }
        }
    }
    // Also copy miniboxd and minibox CLI for tests that invoke them
    let bin_dir = cargo_target_abs.join(target).join("debug");
    for bin_name in &["miniboxd", "minibox"] {
        let src = bin_dir.join(bin_name);
        if src.exists() {
            let dest = tests_dir.join(bin_name);
            std::fs::copy(&src, &dest).with_context(|| format!("copying {bin_name}"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
                    .context("chmod binary")?;
            }
            copied += 1;
        }
    }
    println!("  staged  {copied} binaries into rootfs/tests/");

    // 4. Refresh init scripts (run-tests.sh) then rebuild initramfs with test binaries embedded
    crate::vm_image::install_init_files(&rootfs_dir)?;
    let initrd = vm_dir.join("minibox-initramfs-test.img");
    crate::vm_image::create_initramfs(&rootfs_dir, &initrd, true)?;

    // 5. Create unique serial socket path
    let pid = std::process::id();
    let sock_path = format!("/tmp/minibox-vm-serial-{pid}.sock");
    let serial_arg = format!("unix:{sock_path},server,nowait");

    println!("Starting QEMU VM for tests...");
    println!("  serial socket: {sock_path}");

    // 6. Spawn QEMU
    let mut child = Command::new(platform.qemu_binary())
        .args([
            "-M",
            platform.machine_type(),
            "-cpu",
            "host",
            "-accel",
            platform.accel(),
            "-m",
            "2048",
            "-smp",
            "4",
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

    // 7. Connect to serial socket with retry
    let stream = {
        let mut attempts = 0u32;
        let max_attempts = 50; // 10 seconds
        loop {
            match UnixStream::connect(&sock_path) {
                Ok(s) => break s,
                Err(_) if attempts < max_attempts => {
                    attempts += 1;
                    thread::sleep(Duration::from_millis(200));
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = std::fs::remove_file(&sock_path);
                    bail!(
                        "could not connect to VM serial socket after {}s: {e}",
                        max_attempts / 5
                    );
                }
            }
        }
    };

    // 8. Read lines, print with [vm] prefix, watch for sentinel
    let reader = BufReader::new(stream);
    let mut final_rc: Option<i32> = None;

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

    // 9. Wait for QEMU to exit
    let _ = child.wait();
    let _ = std::fs::remove_file(&sock_path);

    // 10. Evaluate result
    match final_rc {
        Some(0) => {
            println!("All VM tests passed.");
            Ok(())
        }
        Some(n) => bail!("VM tests failed (rc={n})"),
        None => bail!("VM tests did not produce a MINIBOX_TESTS_DONE sentinel — check VM output"),
    }
}
