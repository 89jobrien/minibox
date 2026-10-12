#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! anyhow = "1"
//! ```
//!
//! `qemu-linux-tests` — run the minibox **lib** test binary on a real aarch64
//! Linux kernel, for tests that cannot be exercised on a macOS host.
//!
//! # Why this exists
//!
//! Two independent gaps make the `container` module's unit tests unrunnable on
//! a macOS dev machine:
//!
//! 1. `crates/minibox/src/container/` is gated on `target_os = "linux"`, so
//!    those tests are not compiled into a darwin build at all.
//! 2. They exercise ID-mapped bind mounts, which need privileges and
//!    filesystems a macOS host cannot provide.
//!
//! `cargo xtask test-in-vm` does **not** close the gap:
//! `build_test_script()` in `xtask/src/test_in_vm.rs` collects a hardcoded list
//! of *integration* test binaries only (`integration_tests`, `cli_e2e_tests`,
//! plus the privileged trio). The `minibox` **lib** test binary is not in that
//! list, so no `minibox` unit test ever runs in a VM through the stock path.
//!
//! # How it works
//!
//! 1. Cross-builds the lib test binary for `aarch64-unknown-linux-musl` with
//!    the local musl toolchain. It links statically, so the guest needs no
//!    libc.
//! 2. Extracts a *copy* of the Alpine initramfs from `~/.minibox/vm/boot` into
//!    `target/qemu-linux-tests/tree/`, stages the test binary into it, and
//!    replaces its `init` with the sibling `init` file.
//! 3. Repacks the copy and boots it under `qemu-system-aarch64 -M virt`,
//!    streaming the serial console to stdout.
//!
//! The original boot assets are never modified. Scratch state lives under
//! `target/`, which is gitignored.
//!
//! The test filter is passed on the kernel command line (`mbx_filter=`), so
//! re-running with a different filter neither rebuilds nor repacks.
//!
//! # Known limitation — read before trusting a red result
//!
//! minibox's own `virt` kernel asset does not permit user-namespace creation,
//! and `mount_setattr(MOUNT_ATTR_IDMAP)` is meaningless without one. These
//! three tests therefore **cannot** pass here, on any filesystem, at any
//! privilege level:
//!
//! ```text
//! container::filesystem::tests::bind_mount_tests::apply_bind_mounts_creates_target_dir
//! container::filesystem::tests::bind_mount_tests::apply_bind_mounts_mounts_directory
//! container::filesystem::tests::bind_mount_tests::apply_bind_mounts_read_only
//! ```
//!
//! Ruled out as causes, by measurement inside the guest:
//!
//! | Hypothesis             | Measured                                  |
//! |------------------------|-------------------------------------------|
//! | Insufficient privilege | `CapEff: 000001ffffffffff` (full root)  |
//! | Kernel lockdown        | `lockdown: [none]`                        |
//! | Seccomp                | `Seccomp: 0`, `Seccomp_filters: 0`        |
//! | Filesystem type        | identical `EPERM` on tmpfs/overlay/root   |
//! | `mount`/`bind` broken  | both succeed; only the idmap step fails   |
//! | **No user namespaces** | **`unshare -U` fails**                    |
//!
//! Reproduced identically under both smolvm (Firecracker) and QEMU, which is
//! what ruled out "the VM" as a variable. They need a real distro kernel or a
//! real Linux host; everything else in the lib suite runs here, including the
//! bind-mount path-traversal tests.
//!
//! # Usage
//!
//! ```sh
//! rust-script scripts/qemu-linux-tests.rs                        # whole lib suite
//! rust-script scripts/qemu-linux-tests.rs --filter container::filesystem
//! rust-script scripts/qemu-linux-tests.rs --filter bind_mount_target
//! rust-script scripts/qemu-linux-tests.rs --no-build              # reuse last build
//! rust-script scripts/qemu-linux-tests.rs --pack-only            # just repack
//! ```
//!
//! Requires `qemu` (`brew install qemu`), `aarch64-linux-musl-gcc`
//! (`messense/macos-cross-toolchains`), `cpio`, `gzip`, and VM assets from
//! `cargo xtask build-vm-image`.

use anyhow::{anyhow, bail, Context, Result};
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const TARGET: &str = "aarch64-unknown-linux-musl";
const MUSL_CC: &str = "aarch64-linux-musl-gcc";
const TEST_PKG: &str = "minibox";

/// Parsed command-line options.
struct Opts {
    filter: String,
    build: bool,
    pack_only: bool,
    timeout_secs: u64,
}

/// Walk up from `start` looking for the minibox workspace root.
///
/// Identified by a `Cargo.toml` next to a `scripts/` directory, so this works no
/// matter which directory the script is invoked from.
fn find_repo_root(start: &Path) -> Result<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join("Cargo.toml").is_file() && dir.join("scripts").is_dir() {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!(
                "could not find the minibox repo root above {} \
                 (expected a Cargo.toml beside a scripts/ directory)",
                start.display()
            );
        }
    }
}

/// Directory holding this script's siblings (the `init` template).
///
/// Resolved from the repo root rather than from `argv[0]`, because rust-script
/// compiles to a cached binary and does not preserve the script path.
fn script_dir(root: &Path) -> PathBuf {
    root.join("scripts").join("qemu-linux-tests")
}

fn parse_args() -> Result<Opts> {
    let mut opts = Opts {
        filter: String::new(),
        build: true,
        pack_only: false,
        timeout_secs: 300,
    };
    let mut it = env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--filter" => {
                opts.filter = it.next().ok_or_else(|| anyhow!("--filter needs a value"))?;
            }
            "--no-build" => opts.build = false,
            "--pack-only" => opts.pack_only = true,
            "--timeout" => {
                let v = it
                    .next()
                    .ok_or_else(|| anyhow!("--timeout needs a value"))?;
                opts.timeout_secs = v
                    .parse()
                    .with_context(|| format!("--timeout expects seconds, got {v:?}"))?;
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => bail!("unrecognised argument {other:?} (try --help)"),
        }
    }
    Ok(opts)
}

fn print_help() {
    println!(
        "qemu-linux-tests — run the minibox lib tests on a real aarch64 Linux kernel

  --filter <substring>   run only tests whose name contains <substring>
  --no-build             reuse the existing cross-built test binary
  --pack-only            repack the initramfs but do not boot
  --timeout <seconds>    kill qemu if the guest has not exited by then (default 300)

Reads boot assets from ~/.minibox/vm/boot (never modifies them).
Scratch work lives in target/qemu-linux-tests/.

The guest normally powers itself off. The timeout is a backstop for a guest that
hangs, so a bad image can never wedge the harness.

See scripts/qemu-linux-tests/README.md for the three tests that cannot pass
here (the virt kernel forbids user namespaces) and why."
    );
}

fn main() -> Result<()> {
    let opts = parse_args()?;

    let cwd = env::current_dir()?;
    let root = find_repo_root(&cwd)?;
    let sdir = script_dir(&root);
    let init_template = sdir.join("init");
    if !init_template.is_file() {
        bail!("missing {} (the init template)", init_template.display());
    }

    let work = root.join("target").join("qemu-linux-tests");
    fs::create_dir_all(&work).context("creating the scratch work dir")?;

    let boot = home_dir()?.join(".minibox").join("vm").join("boot");
    for asset in ["vmlinuz-virt", "initramfs-virt"] {
        let p = boot.join(asset);
        if !p.is_file() {
            bail!(
                "missing boot asset {}\nRun `cargo xtask build-vm-image` first.",
                p.display()
            );
        }
    }
    require_on_path("qemu-system-aarch64")?;
    require_on_path("cpio")?;

    if opts.build {
        cross_build(&root)?;
    }

    let test_bin = find_test_binary(&root)?;
    println!("test binary: {}", test_bin.display());

    let initrd = pack_initramfs(&work, &boot, &init_template, &test_bin)?;
    println!("initramfs:   {}", initrd.display());

    if opts.pack_only {
        println!("--pack-only: not booting");
        return Ok(());
    }

    // `-cpu max`, NOT `-cpu host`: several QEMU builds reject "host" on arm64.
    let mut append = String::from("console=ttyAMA0 earlycon=pl011,0x9000000");
    if !opts.filter.is_empty() {
        append.push_str(" mbx_filter=");
        append.push_str(&opts.filter);
    }

    run_qemu_with_timeout(
        &boot.join("vmlinuz-virt"),
        &initrd,
        &append,
        opts.timeout_secs,
    )
}

/// Boot QEMU, streaming the serial console live, and kill it if it outlasts
/// `timeout_secs`.
///
/// Two reasons this is not just `.status()`:
///
///   - Buffered output is useless when diagnosing a guest that will not boot.
///     The console has to appear on *our* stdout as it is produced.
///   - A guest that hangs must not hang the harness. The guest normally powers
///     itself off; if it cannot, this deadline is the backstop.
fn run_qemu_with_timeout(
    kernel: &Path,
    initrd: &Path,
    append: &str,
    timeout_secs: u64,
) -> Result<()> {
    use std::io::Write as _;
    use std::thread;
    use std::time::{Duration, Instant};

    let mut child = Command::new("qemu-system-aarch64")
        .args([
            "-M",
            "virt",
            "-cpu",
            "max",
            "-m",
            "2048",
            "-nographic",
            "-no-reboot",
        ])
        .arg("-kernel")
        .arg(kernel)
        .arg("-initrd")
        .arg(initrd)
        .args(["-append", append])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("running qemu-system-aarch64")?;

    // Reader thread pumps the guest console through to our stdout unbuffered,
    // so a hung or crashing guest is still diagnosable.
    let mut pipe = child.stdout.take().expect("stdout was piped");
    let reader = thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut out = std::io::stdout();
        loop {
            match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let _ = out.write_all(&buf[..n]);
                    let _ = out.flush();
                }
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let _ = reader.join();
                if !status.success() {
                    bail!("qemu exited with {status}");
                }
                return Ok(());
            }
            Ok(None) => {}
            Err(e) => bail!("waiting on qemu: {e}"),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            bail!(
                "qemu did not exit within {timeout_secs}s and was killed.\n\
                 The guest ran but failed to power off, or never reached the tests.\n\
                 Re-run with --pack-only and boot the image by hand to see the console."
            );
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn home_dir() -> Result<PathBuf> {
    env::var("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
}

fn require_on_path(exe: &str) -> Result<()> {
    let ok = Command::new("which")
        .arg(exe)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        match exe {
            "qemu-system-aarch64" => bail!("qemu-system-aarch64 not on PATH (brew install qemu)"),
            "cpio" => bail!("cpio not on PATH (needed to repack the initramfs)"),
            other => bail!("{other} not on PATH"),
        }
    }
    Ok(())
}

fn cross_build(root: &Path) -> Result<()> {
    println!("cross-building {TEST_PKG} lib tests for {TARGET} ...");
    let underscored = TARGET.replace('-', "_");
    let status = Command::new("cargo")
        .args([
            "test", "-p", TEST_PKG, "--lib", "--no-run", "--target", TARGET,
        ])
        .env(format!("CC_{underscored}"), MUSL_CC)
        .env(
            format!(
                "CARGO_TARGET_{}_LINKER",
                TARGET.to_uppercase().replace('-', "_")
            ),
            MUSL_CC,
        )
        .current_dir(root)
        .status()
        .context("running cargo test --no-run")?;

    if !status.success() {
        bail!(
            "cross-build failed (exit {status}). If the linker is missing, install \
             messense/macos-cross-toolchains for {MUSL_CC}."
        );
    }
    Ok(())
}

/// Find the freshly built lib test binary: an ELF file in
/// `target/<triple>/debug/deps/` named `minibox-<hash>`.
fn find_test_binary(root: &Path) -> Result<PathBuf> {
    let deps = root.join("target").join(TARGET).join("debug").join("deps");

    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    let entries = fs::read_dir(&deps)
        .with_context(|| format!("reading {}. Run without --no-build first.", deps.display()))?;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("minibox-") || name.contains('.') {
            continue;
        }
        let path = entry.path();
        if !is_elf(&path) {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(_, p)| p != &path) {
            match &best {
                Some((t, _)) if *t >= mtime => {}
                _ => best = Some((mtime, path)),
            }
        }
    }

    best.map(|(_, p)| p)
        .ok_or_else(|| anyhow!("no ELF test binary found in {}", deps.display()))
}

/// True if the file starts with the ELF magic. Distinguishes a real test binary
/// from the `.d` dep-info files and stale artefacts sharing the prefix.
fn is_elf(path: &Path) -> bool {
    let mut f = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return false;
    }
    magic == [0x7f, b'E', b'L', b'F']
}

fn pack_initramfs(
    work: &Path,
    boot: &Path,
    init_template: &Path,
    test_bin: &Path,
) -> Result<PathBuf> {
    let tree = work.join("tree");
    let gz_src = work.join("initramfs-src.gz");

    println!("extracting a copy of the initramfs ...");
    if tree.exists() {
        fs::remove_dir_all(&tree)?;
    }
    fs::create_dir_all(&tree)?;
    fs::copy(boot.join("initramfs-virt"), &gz_src)?;

    // gunzip -c <src> | cpio -idm
    let extracted = pipe_stdio(
        vec![
            (
                "gunzip",
                vec!["-c".to_string(), gz_src.display().to_string()],
            ),
            ("cpio", vec!["-idm".to_string()]),
        ],
        &tree,
    )?;
    std::mem::drop(extracted);
    fs::remove_file(&gz_src)?;

    println!("staging the test binary and harness init ...");
    fs::copy(test_bin, tree.join("mbx-test")).context("staging mbx-test into the initramfs")?;
    let staged_init = tree.join("init");
    fs::copy(init_template, &staged_init).context("staging the init template")?;
    // The kernel must be able to *execute* /init; a non-executable copy fails
    // with EACCES (-13) and the guest silently drops to an interactive shell.
    // Set the mode explicitly rather than trusting the checkout's bit, since
    // git only preserves it if the file was committed that way.
    fs::set_permissions(&staged_init, fs::Permissions::from_mode(0o755))
        .context("making the staged init executable")?;

    println!("repacking ...");
    // find . -print0 | cpio --null -o --format newc  |  gzip -9 -c
    let packed = pipe_stdio(
        vec![
            (
                "sh",
                vec![
                    "-c".to_string(),
                    // Constant script text; no caller input is interpolated.
                    "find . -print0 | cpio --null -o --format newc".to_string(),
                    "sh".to_string(),
                ],
            ),
            ("gzip", vec!["-9".to_string(), "-c".to_string()]),
        ],
        &tree,
    )?;

    let out = work.join("initramfs.gz");
    fs::write(&out, packed).context("writing the repacked initramfs")?;
    Ok(out)
}

/// Run `cmds` as a pipeline where each stage's stdout feeds the next stage's
/// stdin, and return the final stage's stdout.
fn pipe_stdio(cmds: Vec<(&str, Vec<String>)>, cwd: &Path) -> Result<Vec<u8>> {
    let total = cmds.len();
    anyhow::ensure!(total > 0, "empty pipeline");

    let mut prev: Option<std::process::Child> = None;

    for (i, (exe, args)) in cmds.into_iter().enumerate() {
        let mut cmd = Command::new(exe);
        cmd.args(&args).current_dir(cwd);

        if let Some(mut p) = prev.take() {
            cmd.stdin(Stdio::from(
                p.stdout.take().expect("previous stage was piped"),
            ));
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::inherit());

        let child = cmd
            .spawn()
            .with_context(|| format!("spawning stage {i}: {exe}"))?;
        prev = Some(child);
    }

    let mut last = prev.expect("non-empty pipeline");
    let mut buf = Vec::new();
    last.stdout
        .take()
        .expect("final stage was piped")
        .read_to_end(&mut buf)
        .context("reading pipeline output")?;
    let status = last
        .wait()
        .context("waiting for the final pipeline stage")?;

    if !status.success() {
        bail!("pipeline stage exited with {status}");
    }
    Ok(buf)
}
