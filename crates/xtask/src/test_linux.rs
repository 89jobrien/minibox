//! test_linux — hexagonal architecture for `cargo xtask test-linux`.
//!
//! Three port traits define the pipeline:
//!   Compiler        — cross-compile test binaries to musl
//!   InitramfsBuilder — assemble gzip cpio initramfs from a rootfs directory
//!   VmRunner        — boot QEMU, stream serial, detect sentinel
//!
//! Real adapters delegate to the existing `vm_run` and `test_image` machinery.
//! Mock adapters enable unit tests that run without QEMU, cargo, or a VM image.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------

/// Cross-compile miniboxd + test binaries to a musl target.
pub trait Compiler {
    /// Compile binaries for the given musl target triple.
    /// Outputs land in `cargo_target/<target>/debug/` and `deps/`.
    fn compile(&self, target: &str) -> Result<()>;
}

/// Build a gzip-compressed cpio initramfs from an existing rootfs directory.
pub trait InitramfsBuilder {
    /// Assemble the initramfs at `output_path` using `rootfs_dir` as source.
    fn build(&self, rootfs_dir: &Path, output_path: &Path) -> Result<()>;
}

/// Boot a QEMU VM, stream serial output, and detect the test-done sentinel.
pub trait VmRunner {
    /// Boot the VM with the given kernel, initramfs, and cmdline.
    /// Stream serial output to stdout with a `[vm]` prefix.
    /// Return Ok(()) iff `MINIBOX_TESTS_DONE rc=0` is received before EOF.
    fn run(&self, kernel_path: &Path, initramfs_path: &Path, cmdline: &str) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Real adapters
// ---------------------------------------------------------------------------

/// Compile via `cargo zigbuild`, falling back to `cargo build` with
/// `CC_<target>` / `CARGO_TARGET_<target>_LINKER` env vars pointing at
/// `aarch64-linux-musl-gcc` (or the x86_64 equivalent).
pub struct ZigbuildCompiler {
    /// Crate packages to build as binaries (e.g. `["miniboxd", "mbx"]`).
    pub bin_packages: Vec<String>,
    /// Test packages to compile with `--tests` (e.g. `["miniboxd"]`).
    pub test_packages: Vec<String>,
}

impl ZigbuildCompiler {
    pub fn new(bin_packages: Vec<String>, test_packages: Vec<String>) -> Self {
        Self {
            bin_packages,
            test_packages,
        }
    }

    fn run_cargo_zigbuild(args: &[&str]) -> Result<()> {
        let status = std::process::Command::new("cargo")
            .args(args)
            .status()
            .context("spawning cargo zigbuild")?;
        if !status.success() {
            bail!("cargo zigbuild {:?} failed", args);
        }
        Ok(())
    }

    /// Fallback path when cargo-zigbuild is not installed.
    fn run_cargo_with_musl_gcc(args: &[&str], target: &str) -> Result<()> {
        let cc = musl_gcc_for(target);
        let cc_env = format!("CC_{}", target.replace('-', "_"));
        let linker_env = format!(
            "CARGO_TARGET_{}_LINKER",
            target.to_uppercase().replace('-', "_")
        );
        let status = std::process::Command::new("cargo")
            .args(args)
            .env(&cc_env, &cc)
            .env(&linker_env, &cc)
            .status()
            .context("spawning cargo build")?;
        if !status.success() {
            bail!("cargo build {:?} failed", args);
        }
        Ok(())
    }
}

fn musl_gcc_for(target: &str) -> String {
    if target.starts_with("aarch64") {
        "aarch64-linux-musl-gcc".to_string()
    } else {
        "x86_64-linux-musl-gcc".to_string()
    }
}

fn has_cargo_zigbuild() -> bool {
    std::process::Command::new("cargo")
        .args(["zigbuild", "--version"])
        .output()
        .is_ok_and(|o| o.status.success())
}

impl Compiler for ZigbuildCompiler {
    fn compile(&self, target: &str) -> Result<()> {
        let use_zigbuild = has_cargo_zigbuild();

        // Compile binaries
        for pkg in &self.bin_packages {
            println!("  compiling  {pkg} → {target}");
            if use_zigbuild {
                ZigbuildCompiler::run_cargo_zigbuild(&["zigbuild", "-p", pkg, "--target", target])?;
            } else {
                ZigbuildCompiler::run_cargo_with_musl_gcc(
                    &["build", "-p", pkg, "--target", target],
                    target,
                )?;
            }
        }

        // Compile test binaries
        for pkg in &self.test_packages {
            println!("  compiling tests {pkg} → {target}");
            if use_zigbuild {
                ZigbuildCompiler::run_cargo_zigbuild(&[
                    "zigbuild", "--tests", "-p", pkg, "--target", target,
                ])?;
            } else {
                ZigbuildCompiler::run_cargo_with_musl_gcc(
                    &["test", "--no-run", "-p", pkg, "--target", target],
                    target,
                )?;
            }
        }

        Ok(())
    }
}

/// Build gzip-compressed cpio initramfs by delegating to `vm_image`.
pub struct CpioInitramfsBuilder;

impl InitramfsBuilder for CpioInitramfsBuilder {
    fn build(&self, rootfs_dir: &Path, output_path: &Path) -> Result<()> {
        crate::vm_image::create_initramfs(rootfs_dir, output_path, true)
            .with_context(|| format!("building initramfs from {}", rootfs_dir.display()))
    }
}

/// Boot QEMU and watch serial output for the `MINIBOX_TESTS_DONE rc=` sentinel.
/// Wraps the existing `vm_run::VmRunner` machinery.
pub struct QemuVmRunner {
    platform: crate::vm_run::HostPlatform,
    /// VM directory (contains `boot/vmlinuz-virt`, etc.).
    vm_dir: PathBuf,
}

impl QemuVmRunner {
    pub fn new(platform: crate::vm_run::HostPlatform, vm_dir: PathBuf) -> Self {
        Self { platform, vm_dir }
    }
}

impl VmRunner for QemuVmRunner {
    fn run(&self, _kernel_path: &Path, _initramfs_path: &Path, cmdline: &str) -> Result<()> {
        use std::io::BufRead;

        let inner = crate::vm_run::VmRunner::new(
            self.platform.clone(),
            self.vm_dir.clone(),
            PathBuf::from("target"), // not used — initramfs already built
        );

        let handle = inner.spawn_vm(cmdline)?;
        let stream = handle.connect_serial()?;
        let reader = std::io::BufReader::new(stream);
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
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

/// Run the full test-linux pipeline using the provided adapters.
///
/// Steps:
///   1. Cross-compile via `compiler`
///   2. Stage binaries into `rootfs/tests/`
///   3. Install init files
///   4. Build initramfs via `initramfs_builder`
///   5. Boot VM and stream tests via `vm_runner`
pub fn run_pipeline(
    compiler: &dyn Compiler,
    initramfs_builder: &dyn InitramfsBuilder,
    vm_runner: &dyn VmRunner,
    target: &str,
    vm_dir: &Path,
    cargo_target: &Path,
    kernel_path: &Path,
) -> Result<()> {
    // 1. Compile
    println!("[1/4] cross-compiling for {target} …");
    compiler.compile(target)?;

    // 2. Stage binaries
    println!("[2/4] staging binaries …");
    let rootfs = vm_dir.join("rootfs");
    let tests_dir = rootfs.join("tests");
    std::fs::create_dir_all(&tests_dir).context("creating rootfs/tests")?;

    let deps_dir = cargo_target.join(target).join("debug").join("deps");
    let bin_dir = cargo_target.join(target).join("debug");

    let test_suites = &[
        "cgroup_tests",
        "e2e_tests",
        "integration_tests",
        "sandbox_tests",
    ];
    for suite in test_suites {
        if let Ok(entries) = std::fs::read_dir(&deps_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if !name_str.starts_with(suite) || name_str.contains('.') {
                    continue;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if entry
                        .metadata()
                        .is_ok_and(|m| m.permissions().mode() & 0o111 == 0)
                    {
                        continue;
                    }
                }
                let dest = tests_dir.join(&*name_str);
                std::fs::copy(entry.path(), &dest)
                    .with_context(|| format!("copying {name_str}"))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
                        .context("chmod test binary")?;
                }
                println!("  staged  {name_str}");
                break;
            }
        }
    }

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
        }
    }

    // 3. Install init files
    println!("[3/4] installing init …");
    crate::vm_image::install_init_files(&rootfs)?;

    // 4. Build initramfs
    let initramfs_path = vm_dir.join("minibox-initramfs-test.img");
    println!("[4/4] building initramfs …");
    initramfs_builder.build(&rootfs, &initramfs_path)?;

    // 5. Boot and run tests
    println!("Starting QEMU VM for tests…");
    let cmdline = "rdinit=/sbin/init console=ttyAMA0,115200 minibox.mode=test";
    vm_runner.run(kernel_path, &initramfs_path, cmdline)?;

    Ok(())
}

/// Entry point: wire real adapters and run the pipeline.
///
/// Replaces the old `test_image::test_linux` stub.
pub fn test_linux() -> Result<()> {
    let platform = crate::vm_run::HostPlatform::detect()?;
    let target = platform.musl_target();
    let vm_dir = crate::vm_image::default_vm_dir();
    let cargo_target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target"));

    let kernel_path = vm_dir.join("boot").join("vmlinuz-virt");
    if !kernel_path.exists() {
        bail!(
            "kernel not found at {}; run `cargo xtask build-vm-image` first",
            kernel_path.display()
        );
    }

    let compiler = ZigbuildCompiler::new(
        vec!["miniboxd".to_string(), "mbx".to_string()],
        vec!["miniboxd".to_string()],
    );
    let initramfs_builder = CpioInitramfsBuilder;
    let vm_runner = QemuVmRunner::new(platform, vm_dir.clone());

    run_pipeline(
        &compiler,
        &initramfs_builder,
        &vm_runner,
        target,
        &vm_dir,
        &cargo_target,
        &kernel_path,
    )
}

// ---------------------------------------------------------------------------
// Mock adapters (for unit tests only)
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod mocks {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Records `compile()` calls and optionally injects a failure.
    pub struct MockCompiler {
        pub calls: Arc<Mutex<Vec<String>>>,
        pub fail: bool,
    }

    impl MockCompiler {
        pub fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(vec![])),
                fail: false,
            }
        }
        pub fn failing() -> Self {
            Self {
                fail: true,
                ..Self::new()
            }
        }
    }

    impl Compiler for MockCompiler {
        fn compile(&self, target: &str) -> Result<()> {
            if self.fail {
                bail!("mock compiler: simulated failure");
            }
            self.calls.lock().unwrap().push(target.to_string());
            Ok(())
        }
    }

    /// Records `build()` calls and optionally injects a failure.
    pub struct MockInitramfsBuilder {
        pub calls: Arc<Mutex<Vec<(PathBuf, PathBuf)>>>,
        pub fail: bool,
    }

    impl MockInitramfsBuilder {
        pub fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(vec![])),
                fail: false,
            }
        }
        pub fn failing() -> Self {
            Self {
                fail: true,
                ..Self::new()
            }
        }
    }

    impl InitramfsBuilder for MockInitramfsBuilder {
        fn build(&self, rootfs_dir: &Path, output_path: &Path) -> Result<()> {
            if self.fail {
                bail!("mock initramfs builder: simulated failure");
            }
            // Write a placeholder so callers that stat the file don't fail.
            std::fs::write(output_path, b"mock-initramfs").ok();
            self.calls
                .lock()
                .unwrap()
                .push((rootfs_dir.to_path_buf(), output_path.to_path_buf()));
            Ok(())
        }
    }

    /// Records `run()` calls and optionally injects a failure.
    pub struct MockVmRunner {
        pub calls: Arc<Mutex<Vec<String>>>,
        pub fail: bool,
    }

    impl MockVmRunner {
        pub fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(vec![])),
                fail: false,
            }
        }
        pub fn failing() -> Self {
            Self {
                fail: true,
                ..Self::new()
            }
        }
    }

    impl VmRunner for MockVmRunner {
        fn run(&self, _kernel: &Path, _initramfs: &Path, cmdline: &str) -> Result<()> {
            if self.fail {
                bail!("mock vm runner: simulated failure");
            }
            self.calls.lock().unwrap().push(cmdline.to_string());
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mocks::*;
    use tempfile::tempdir;

    /// Helper: create the directory structure pipeline expects so staging
    /// doesn't fail with "no such file".
    fn setup_vm_dirs(vm_dir: &Path, cargo_target: &Path, target: &str) {
        let rootfs = vm_dir.join("rootfs");
        std::fs::create_dir_all(rootfs.join("tests")).ok();
        let deps_dir = cargo_target.join(target).join("debug").join("deps");
        std::fs::create_dir_all(&deps_dir).ok();
        let bin_dir = cargo_target.join(target).join("debug");
        std::fs::create_dir_all(&bin_dir).ok();
        // kernel placeholder
        let boot_dir = vm_dir.join("boot");
        std::fs::create_dir_all(&boot_dir).ok();
        std::fs::write(boot_dir.join("vmlinuz-virt"), b"kernel").ok();
    }

    #[test]
    fn pipeline_records_all_adapter_calls_on_success() {
        let tmp = tempdir().unwrap();
        let vm_dir = tmp.path().join("vm");
        let cargo_target = tmp.path().join("target");
        let target = "aarch64-unknown-linux-musl";
        setup_vm_dirs(&vm_dir, &cargo_target, target);

        let compiler = MockCompiler::new();
        let initramfs_builder = MockInitramfsBuilder::new();
        let vm_runner = MockVmRunner::new();
        let kernel = vm_dir.join("boot").join("vmlinuz-virt");

        // install_init_files will look for sbin/init — patch vm_image by
        // ensuring the function tolerates an already-populated rootfs.
        // We call vm_image::install_init_files during the pipeline; write
        // the expected files manually so it is a no-op.
        let sbin = vm_dir.join("rootfs").join("sbin");
        std::fs::create_dir_all(&sbin).ok();
        std::fs::write(sbin.join("init"), b"#!/bin/sh\n").ok();

        let result = run_pipeline(
            &compiler,
            &initramfs_builder,
            &vm_runner,
            target,
            &vm_dir,
            &cargo_target,
            &kernel,
        );
        assert!(result.is_ok(), "pipeline should succeed: {result:?}");

        let compile_calls = compiler.calls.lock().unwrap();
        assert_eq!(*compile_calls, vec![target.to_string()]);

        let initramfs_calls = initramfs_builder.calls.lock().unwrap();
        assert_eq!(initramfs_calls.len(), 1);

        let vm_calls = vm_runner.calls.lock().unwrap();
        assert_eq!(vm_calls.len(), 1);
        assert!(vm_calls[0].contains("minibox.mode=test"));
    }

    #[test]
    fn pipeline_aborts_on_compiler_failure() {
        let tmp = tempdir().unwrap();
        let vm_dir = tmp.path().join("vm");
        let cargo_target = tmp.path().join("target");
        let target = "aarch64-unknown-linux-musl";
        setup_vm_dirs(&vm_dir, &cargo_target, target);

        let compiler = MockCompiler::failing();
        let initramfs_builder = MockInitramfsBuilder::new();
        let vm_runner = MockVmRunner::new();
        let kernel = vm_dir.join("boot").join("vmlinuz-virt");

        let result = run_pipeline(
            &compiler,
            &initramfs_builder,
            &vm_runner,
            target,
            &vm_dir,
            &cargo_target,
            &kernel,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("mock compiler"));

        // InitramfsBuilder and VmRunner must NOT have been called.
        assert!(initramfs_builder.calls.lock().unwrap().is_empty());
        assert!(vm_runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn pipeline_aborts_on_initramfs_failure() {
        let tmp = tempdir().unwrap();
        let vm_dir = tmp.path().join("vm");
        let cargo_target = tmp.path().join("target");
        let target = "aarch64-unknown-linux-musl";
        setup_vm_dirs(&vm_dir, &cargo_target, target);

        let sbin = vm_dir.join("rootfs").join("sbin");
        std::fs::create_dir_all(&sbin).ok();
        std::fs::write(sbin.join("init"), b"#!/bin/sh\n").ok();

        let compiler = MockCompiler::new();
        let initramfs_builder = MockInitramfsBuilder::failing();
        let vm_runner = MockVmRunner::new();
        let kernel = vm_dir.join("boot").join("vmlinuz-virt");

        let result = run_pipeline(
            &compiler,
            &initramfs_builder,
            &vm_runner,
            target,
            &vm_dir,
            &cargo_target,
            &kernel,
        );
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("mock initramfs"),
            "error should mention initramfs"
        );
        assert!(vm_runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn pipeline_aborts_on_vm_runner_failure() {
        let tmp = tempdir().unwrap();
        let vm_dir = tmp.path().join("vm");
        let cargo_target = tmp.path().join("target");
        let target = "aarch64-unknown-linux-musl";
        setup_vm_dirs(&vm_dir, &cargo_target, target);

        let sbin = vm_dir.join("rootfs").join("sbin");
        std::fs::create_dir_all(&sbin).ok();
        std::fs::write(sbin.join("init"), b"#!/bin/sh\n").ok();

        let compiler = MockCompiler::new();
        let initramfs_builder = MockInitramfsBuilder::new();
        let vm_runner = MockVmRunner::failing();
        let kernel = vm_dir.join("boot").join("vmlinuz-virt");

        let result = run_pipeline(
            &compiler,
            &initramfs_builder,
            &vm_runner,
            target,
            &vm_dir,
            &cargo_target,
            &kernel,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("mock vm runner"));
    }

    #[test]
    fn zigbuild_compiler_constructs_correctly() {
        let c = ZigbuildCompiler::new(vec!["miniboxd".to_string()], vec!["miniboxd".to_string()]);
        assert_eq!(c.bin_packages, vec!["miniboxd"]);
        assert_eq!(c.test_packages, vec!["miniboxd"]);
    }

    #[test]
    fn musl_gcc_for_aarch64() {
        assert_eq!(
            musl_gcc_for("aarch64-unknown-linux-musl"),
            "aarch64-linux-musl-gcc"
        );
    }

    #[test]
    fn musl_gcc_for_x86_64() {
        assert_eq!(
            musl_gcc_for("x86_64-unknown-linux-musl"),
            "x86_64-linux-musl-gcc"
        );
    }
}
