//! test_linux — hexagonal architecture for `cargo xtask test-linux`.
//!
//! Three port traits define the pipeline:
//!   Compiler         — cross-compile test binaries to musl
//!   InitramfsBuilder — assemble gzip cpio initramfs from a rootfs directory
//!   VmRunner         — boot VM or run container, stream output, detect sentinel
//!
//! Real adapters: ZigbuildCompiler, CpioInitramfsBuilder, SmolvmRunner.

use anyhow::{Context, Result, bail};
use std::path::Path;

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

/// Boot a micro-VM or container and stream test output.
pub trait VmRunner {
    /// Boot the VM with the given kernel and initramfs, or run a container image.
    /// Return Ok(()) iff the test suite exits with status 0.
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

/// Build a gzip-compressed cpio initramfs from a rootfs directory.
///
/// Shells out to `find | cpio --null -o --format=newc | gzip` (available on any Linux CI).
pub struct CpioInitramfsBuilder;

impl InitramfsBuilder for CpioInitramfsBuilder {
    fn build(&self, rootfs_dir: &Path, output_path: &Path) -> Result<()> {
        use std::process::{Command, Stdio};

        let find = Command::new("find")
            .arg(".")
            .arg("-print0")
            .current_dir(rootfs_dir)
            .stdout(Stdio::piped())
            .spawn()
            .context("spawning find")?;

        let cpio = Command::new("cpio")
            .args(["--null", "-o", "--format=newc"])
            .stdin(find.stdout.context("find stdout")?)
            .stdout(Stdio::piped())
            .spawn()
            .context("spawning cpio")?;

        let gzip_out = std::fs::File::create(output_path)
            .with_context(|| format!("creating {}", output_path.display()))?;

        let mut gzip = Command::new("gzip")
            .stdin(cpio.stdout.context("cpio stdout")?)
            .stdout(gzip_out)
            .spawn()
            .context("spawning gzip")?;

        let status = gzip.wait().context("waiting for gzip")?;
        if !status.success() {
            bail!("cpio/gzip pipeline failed building initramfs");
        }
        Ok(())
    }
}

/// Run tests via `minibox run --privileged <image_name> -- /run-tests.sh`.
///
/// `kernel_path` and `initramfs_path` are unused; the container image is expected to have
/// been built and loaded via `cargo xtask build-test-image` before invoking this runner.
pub struct SmolvmRunner {
    /// OCI image reference to run, e.g. `minibox-tester:latest`.
    pub image_name: String,
}

impl VmRunner for SmolvmRunner {
    fn run(&self, _kernel_path: &Path, _initramfs_path: &Path, _cmdline: &str) -> Result<()> {
        println!(
            "SmolvmRunner: minibox run --privileged {} -- /run-tests.sh",
            self.image_name
        );
        let status = std::process::Command::new("minibox")
            .args(["run", "--privileged"])
            .arg(&self.image_name)
            .args(["--", "/run-tests.sh"])
            .status()
            .context("spawning minibox run")?;
        if !status.success() {
            bail!("minibox run {} exited non-zero", self.image_name);
        }
        Ok(())
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
        "system_tests",
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

    // 3/4. Build initramfs
    let initramfs_path = vm_dir.join("minibox-initramfs-test.img");
    println!("[4/4] building initramfs …");
    initramfs_builder.build(&rootfs, &initramfs_path)?;

    // 5. Boot and run tests
    println!("Starting QEMU VM for tests…");
    let cmdline = "rdinit=/sbin/init console=ttyAMA0,115200 minibox.mode=test";
    vm_runner.run(kernel_path, &initramfs_path, cmdline)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Mock adapters (for unit tests only)
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod mocks {
    use super::*;
    use std::path::PathBuf;
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
    fn smolvm_runner_constructs_correctly() {
        let r = SmolvmRunner {
            image_name: "minibox-tester:latest".to_string(),
        };
        assert_eq!(r.image_name, "minibox-tester:latest");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn cpio_initramfs_builder_creates_output_file() {
        let tmp = tempdir().unwrap();
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(rootfs.join("bin")).unwrap();
        std::fs::write(rootfs.join("bin/sh"), b"#!/bin/sh").unwrap();
        let output = tmp.path().join("initramfs.img");
        let builder = CpioInitramfsBuilder;
        let result = builder.build(&rootfs, &output);
        assert!(result.is_ok(), "builder failed: {result:?}");
        assert!(output.exists(), "initramfs.img should be created");
        assert!(output.metadata().unwrap().len() > 0, "should be non-empty");
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
