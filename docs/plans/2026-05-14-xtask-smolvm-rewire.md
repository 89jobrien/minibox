# Plan: xtask smolvm re-wire — cas overlay + test_linux adapters

## Goal

Remove stale `#![allow(dead_code)]` suppressions from `xtask/src/cas.rs` and
`xtask/src/test_linux.rs` by deleting dead QEMU-era code from `cas.rs` and wiring real
`CpioInitramfsBuilder` and `SmolvmRunner` adapters into `test_linux.rs`, then wiring the
`test-linux` subcommand in `main.rs` to call `run_pipeline`. Closes #357.

## Architecture

- Crates affected: `xtask` only
- New types:
  - `xtask/src/test_linux.rs` → `CpioInitramfsBuilder` (implements `InitramfsBuilder`)
  - `xtask/src/test_linux.rs` → `SmolvmRunner` (implements `VmRunner`)
- Data flow (test-linux):
  `ZigbuildCompiler` → stage binaries into rootfs → `CpioInitramfsBuilder` → cpio.gz →
  `SmolvmRunner` (calls `smolvm` CLI with kernel + initramfs) → exit code

## Tech Stack

- Rust 2024, xtask binary (no external crate additions needed)
- `CpioInitramfsBuilder` shells out to `find | cpio | gzip` (standard Linux tools)
- `SmolvmRunner` shells out to `smolvm run` or falls back to `minibox run --adapter=smolvm`

## Tasks

### Task 1: Remove dead code from cas.rs

**Crate**: `xtask`
**File(s)**: `xtask/src/cas.rs`
**Run**: `cargo check --manifest-path xtask/Cargo.toml`

The functions `write_cas_refs` and `read_refs` are QEMU-initramfs specific and have no
callers after the vm_image pipeline was dropped in #306. Remove them.

1. Write failing test (none needed — this is a deletion; clippy will flag if dead code
   warning fires after allow removal):
   Verify `cargo clippy -p xtask -- -D warnings` currently passes with `#![allow(dead_code)]`.

2. Implement:
   - Remove lines 7 (`#![allow(dead_code)] // TODO(#357): ...`) from `cas.rs`
   - Remove functions `write_cas_refs` (lines 157–178) and `read_refs` (lines 124–152)
   - Remove tests `write_cas_refs_produces_tab_separated_file`,
     `write_cas_refs_skips_when_no_refs`, and `read_refs_returns_sorted_pairs` from the
     `#[cfg(test)]` module

3. Verify:
   ```
   cargo check --manifest-path xtask/Cargo.toml          → clean
   cargo clippy --manifest-path xtask/Cargo.toml -- -D warnings  → zero warnings
   cargo test --manifest-path xtask/Cargo.toml            → all green
   ```

4. Run: `git branch --show-current`
   Commit: `git commit -m "fix(xtask): remove dead QEMU-era write_cas_refs/read_refs from cas.rs"`

---

### Task 2: Add CpioInitramfsBuilder real adapter

**Crate**: `xtask`
**File(s)**: `xtask/src/test_linux.rs`
**Run**: `cargo test --manifest-path xtask/Cargo.toml`

1. Write failing test:
   ```rust
   #[test]
   fn cpio_initramfs_builder_creates_output_file() {
       let tmp = tempdir().unwrap();
       let rootfs = tmp.path().join("rootfs");
       std::fs::create_dir_all(rootfs.join("bin")).unwrap();
       std::fs::write(rootfs.join("bin/sh"), b"#!/bin/sh").unwrap();
       let output = tmp.path().join("initramfs.img");
       let builder = CpioInitramfsBuilder;
       // Only run on Linux; cpio -o --format=newc is a Linux tool
       #[cfg(target_os = "linux")]
       {
           let result = builder.build(&rootfs, &output);
           assert!(result.is_ok(), "builder failed: {result:?}");
           assert!(output.exists(), "initramfs.img should be created");
           assert!(output.metadata().unwrap().len() > 0, "should be non-empty");
       }
   }
   ```
   Run: `cargo test --manifest-path xtask/Cargo.toml -- cpio_initramfs_builder`
   Expected: FAIL (type doesn't exist yet)

2. Implement — add after `ZigbuildCompiler` impl block in `test_linux.rs`:
   ```rust
   /// Build a gzip-compressed cpio initramfs using standard POSIX `find` + `cpio`.
   ///
   /// Requires `find`, `cpio`, and `gzip` on PATH (Linux CI has these by default).
   pub struct CpioInitramfsBuilder;

   impl InitramfsBuilder for CpioInitramfsBuilder {
       fn build(&self, rootfs_dir: &Path, output_path: &Path) -> Result<()> {
           use std::process::{Command, Stdio};

           // find . -print0 | cpio --null -o --format=newc | gzip > output_path
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
   ```

3. Verify:
   ```
   cargo test --manifest-path xtask/Cargo.toml -- cpio_initramfs_builder  → passes
   cargo clippy --manifest-path xtask/Cargo.toml -- -D warnings            → zero warnings
   ```

4. Run: `git branch --show-current`
   Commit: `git commit -m "feat(xtask): add CpioInitramfsBuilder real adapter"`

---

### Task 3: Add SmolvmRunner real adapter + update VmRunner doc

**Crate**: `xtask`
**File(s)**: `xtask/src/test_linux.rs`
**Run**: `cargo test --manifest-path xtask/Cargo.toml`

1. Write failing test:
   ```rust
   #[test]
   fn smolvm_runner_constructs_correctly() {
       let r = SmolvmRunner { image_name: "minibox-tester:latest".to_string() };
       assert_eq!(r.image_name, "minibox-tester:latest");
   }
   ```
   Run: `cargo test --manifest-path xtask/Cargo.toml -- smolvm_runner_constructs`
   Expected: FAIL (type doesn't exist yet)

2. Implement — update `VmRunner` doc and add `SmolvmRunner` after `CpioInitramfsBuilder`:

   Update `VmRunner` trait doc from:
   ```rust
   /// Boot a QEMU VM, stream serial output, and detect the test-done sentinel.
   pub trait VmRunner {
       /// Boot the VM with the given kernel, initramfs, and cmdline.
       /// Stream serial output to stdout with a `[vm]` prefix.
       /// Return Ok(()) iff `MINIBOX_TESTS_DONE rc=0` is received before EOF.
       fn run(&self, kernel_path: &Path, initramfs_path: &Path, cmdline: &str) -> Result<()>;
   }
   ```
   to:
   ```rust
   /// Boot a micro-VM or container and stream test output.
   pub trait VmRunner {
       /// Boot the VM with the given kernel and initramfs, or run a container image.
       /// Return Ok(()) iff the test suite exits with status 0.
       fn run(&self, kernel_path: &Path, initramfs_path: &Path, cmdline: &str) -> Result<()>;
   }
   ```

   Add `SmolvmRunner`:
   ```rust
   /// Run tests via `minibox run --privileged <image_name> -- /run-tests.sh`.
   ///
   /// `kernel_path` and `initramfs_path` are unused; the container image identified
   /// by `image_name` is expected to have already been built and loaded via
   /// `cargo xtask build-test-image`.
   pub struct SmolvmRunner {
       /// OCI image reference to run, e.g. `minibox-tester:latest`.
       pub image_name: String,
   }

   impl VmRunner for SmolvmRunner {
       fn run(&self, _kernel_path: &Path, _initramfs_path: &Path, _cmdline: &str) -> Result<()> {
           println!("SmolvmRunner: minibox run --privileged {} -- /run-tests.sh", self.image_name);
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
   ```

3. Verify:
   ```
   cargo test --manifest-path xtask/Cargo.toml -- smolvm_runner_constructs  → passes
   cargo clippy --manifest-path xtask/Cargo.toml -- -D warnings              → zero warnings
   ```

4. Run: `git branch --show-current`
   Commit: `git commit -m "feat(xtask): add SmolvmRunner adapter, update VmRunner doc"`

---

### Task 4: Wire test-linux in main.rs + remove #![allow(dead_code)]

**Crate**: `xtask`
**File(s)**: `xtask/src/main.rs`, `xtask/src/test_linux.rs`
**Run**: `cargo check --manifest-path xtask/Cargo.toml`

1. No failing test needed — this wires a CLI subcommand. Verify current state compiles
   (test-linux bails, dead_code allow present).

2. Implement:

   In `test_linux.rs`, remove `#![allow(dead_code)]` (line 10) and update the module
   doc to remove the TODO:
   ```rust
   //! test_linux — hexagonal architecture for `cargo xtask test-linux`.
   //!
   //! Three port traits define the pipeline:
   //!   Compiler         — cross-compile test binaries to musl
   //!   InitramfsBuilder — assemble gzip cpio initramfs from a rootfs directory
   //!   VmRunner         — boot VM or run container, stream output, detect sentinel
   //!
   //! Real adapters: ZigbuildCompiler, CpioInitramfsBuilder, SmolvmRunner.
   ```

   In `main.rs`, replace:
   ```rust
   Some("test-linux") => bail!("test-linux is not yet implemented for smolvm; see #306"),
   ```
   with:
   ```rust
   Some("test-linux") => {
       let target_base = std::env::var("CARGO_TARGET_DIR")
           .map(std::path::PathBuf::from)
           .unwrap_or_else(|_| root.join("target"));
       let vm_dir = dirs::home_dir()
           .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
           .join(".minibox")
           .join("vm");
       // Kernel is expected at vm_dir/boot/vmlinuz-virt (from smolvm image cache).
       let kernel = vm_dir.join("boot").join("vmlinuz-virt");

       let compiler = test_linux::ZigbuildCompiler::new(
           vec!["miniboxd".to_string(), "mbx".to_string()],
           vec!["miniboxd".to_string()],
       );
       let initramfs_builder = test_linux::CpioInitramfsBuilder;
       let vm_runner = test_linux::SmolvmRunner {
           image_name: "minibox-tester:latest".to_string(),
       };

       test_linux::run_pipeline(
           &compiler,
           &initramfs_builder,
           &vm_runner,
           "aarch64-unknown-linux-musl",
           &vm_dir,
           &target_base,
           &kernel,
       )
   }
   ```

   Also add `dirs` to xtask's `Cargo.toml` if not already present (check first).

3. Verify:
   ```
   cargo check --manifest-path xtask/Cargo.toml               → clean
   cargo clippy --manifest-path xtask/Cargo.toml -- -D warnings → zero warnings
   cargo test --manifest-path xtask/Cargo.toml                → all green
   ```

4. Run: `git branch --show-current`
   Commit: `git commit -m "feat(xtask): wire test-linux subcommand for smolvm, remove dead_code allows fixes #357"`

---

## Quality Rules

- No placeholders — every code block above is copy-paste ready
- `dirs` crate is already a dependency of xtask (used in `cas.rs::default_overlay_dir`)
- Task 1 (deletion) has no TDD cycle — clippy gates the removal instead
- Tasks 2–4 follow TDD: failing test → implement → green → commit
- No new crate dependencies required

## Pre-Save Checklist

- [x] Every requirement in #357 maps to a task
- [x] No placeholders or vague directives
- [x] Type names consistent (`CpioInitramfsBuilder`, `SmolvmRunner`) across all tasks
- [x] Each task is 2–5 minutes of focused work
- [x] Each task ends with a commit
