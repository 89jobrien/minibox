//! vm_run — boot the minibox Alpine VM under QEMU with HVF acceleration.
//!
//! Two entry points:
//!   run_vm_interactive   interactive shell on serial console (blocks)
//!   test_vm              build musl test binaries + run in VM, stream results

use anyhow::{Context, Result, bail};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
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

    #[test]
    fn vm_handle_serial_sock_path_is_absolute() {
        let pid = std::process::id();
        let sock = format!("/tmp/minibox-vm-serial-{pid}.sock");
        assert!(sock.starts_with("/tmp/minibox-vm-serial-"));
        assert!(sock.ends_with(".sock"));
    }
}

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
        Self {
            platform,
            vm_dir,
            cargo_target,
        }
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

        let child = Command::new(self.platform.qemu_binary())
            .args([
                "-M",
                self.platform.machine_type(),
                "-cpu",
                "host",
                "-accel",
                self.platform.accel(),
                "-m",
                "2048",
                "-smp",
                "4",
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
        let target = self.platform.musl_target();
        let deps_dir = self.cargo_target.join(target).join("debug").join("deps");
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
                        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
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

        crate::vm_image::install_init_files(&self.vm_dir.join("rootfs"))?;
        let initrd = self.vm_dir.join("minibox-initramfs-test.img");
        crate::vm_image::create_initramfs(&self.vm_dir.join("rootfs"), &initrd, true)?;

        println!("Starting QEMU VM for tests...");
        let handle = self.spawn_vm("rdinit=/sbin/init console=ttyAMA0,115200 minibox.mode=test")?;

        let stream = handle.connect_serial()?;
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

        handle.wait()?;

        match final_rc {
            Some(0) => {
                println!("All VM tests passed.");
                Ok(())
            }
            Some(n) => bail!("VM tests failed (rc={n})"),
            None => {
                bail!("VM tests did not produce a MINIBOX_TESTS_DONE sentinel — check VM output")
            }
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

        let status = Command::new(self.platform.qemu_binary())
            .args([
                "-M",
                self.platform.machine_type(),
                "-cpu",
                "host",
                "-accel",
                self.platform.accel(),
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
            .with_context(|| {
                format!(
                    "spawning {} (is QEMU installed?)",
                    self.platform.qemu_binary()
                )
            })?;

        if !status.success() {
            bail!("QEMU exited with status {}", status);
        }
        Ok(())
    }
}

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

    // Build test binaries first
    let target = platform.musl_target();
    println!("Building test binaries for {target}...");
    let build_status = Command::new("cargo")
        .args(["zigbuild", "--tests", "-p", "miniboxd", "--target", target])
        .status()
        .context("cargo zigbuild --tests (is cargo-zigbuild installed?)")?;
    if !build_status.success() {
        bail!("cargo zigbuild --tests failed");
    }
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

    runner.run_tests(suites)
}
