#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::redundant_field_names,
    clippy::uninlined_format_args,
    clippy::redundant_clone,
    clippy::redundant_closure,
    clippy::single_char_pattern,
    clippy::unwrap_in_result,
    clippy::collapsible_if,
    clippy::match_same_arms,
    clippy::only_used_in_recursion,
    clippy::used_underscore_binding,
    clippy::map_unwrap_or,
    clippy::manual_assert,
    clippy::as_ptr_cast_mut,
    clippy::ptr_as_ptr,
    clippy::must_use_candidate,
    clippy::used_underscore_items,
    clippy::missing_const_for_fn,
    clippy::manual_string_new,
    clippy::semicolon_if_nothing_returned,
    clippy::unreadable_literal,
    clippy::default_constructed_unit_structs,
    clippy::ref_as_ptr,
    clippy::allow_attributes_without_reason,
    clippy::redundant_closure_for_method_calls,
    clippy::needless_raw_string_hashes,
    clippy::manual_is_variant_and,
    clippy::ignore_without_reason,
    clippy::default_trait_access,
    clippy::cast_lossless,
    clippy::match_wild_err_arm,
    clippy::format_push_string,
    clippy::bool_assert_comparison,
    clippy::struct_excessive_bools
)]
//! Security regression tests for tar layer extraction and path validation.
//!
//! These tests guard invariants established by security fixes in commits
//! `8ea4f73` and `2fc7036`. Each test is named after the specific attack
//! vector it prevents. If any of these tests start failing it means a
//! security-critical invariant has been broken.
//!
//! # Invariants under test
//!
//! 1. **Zip Slip / path traversal** — tar entries with `..` components must
//!    be rejected before touching the filesystem (commit `8ea4f73`).
//! 2. **Device node extraction** — block and character device entries must
//!    be rejected outright; extracting them would allow an attacker to access
//!    host hardware devices from inside the container (commit `8ea4f73`).
//! 3. **Absolute symlink host leakage** — absolute symlink targets (e.g.
//!    `/etc/shadow`) are rewritten to relative paths so they resolve correctly
//!    after `pivot_root` without pointing into the host filesystem during
//!    extraction. Targets that still contain `..` after relativisation are
//!    rejected (commit `2fc7036`).
//! 4. **Setuid/setgid bit stripping** — special permission bits (04000,
//!    02000) are stripped from regular file modes before extraction, preventing
//!    privilege escalation via setuid binaries planted in an OCI layer
//!    (commit `2fc7036`).

use flate2::{Compression, write::GzEncoder};
use minibox::image::layer::extract_layer;
use std::io::Write;
use tar::{Builder, EntryType, Header};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Tar archive builders
// ---------------------------------------------------------------------------

/// Build a tar.gz containing a single regular file.
fn tar_gz_regular_file(name: &str, content: &[u8], mode: u32) -> Vec<u8> {
    let gz = GzEncoder::new(Vec::new(), Compression::default());
    let mut ar = Builder::new(gz);
    let mut h = Header::new_gnu();
    h.set_path(name).expect("unwrap in test");
    h.set_size(content.len() as u64);
    h.set_entry_type(EntryType::Regular);
    h.set_mode(mode);
    h.set_cksum();
    ar.append(&h, content).expect("unwrap in test");
    ar.into_inner()
        .expect("unwrap in test")
        .finish()
        .expect("unwrap in test")
}

/// Build a tar.gz containing a device node entry.
fn tar_gz_device_node(name: &str, kind: EntryType) -> Vec<u8> {
    let gz = GzEncoder::new(Vec::new(), Compression::default());
    let mut ar = Builder::new(gz);
    let mut h = Header::new_gnu();
    h.set_path(name).expect("unwrap in test");
    h.set_size(0);
    h.set_entry_type(kind);
    h.set_mode(0o644);
    h.set_cksum();
    ar.append(&h, &[][..]).expect("unwrap in test");
    ar.into_inner()
        .expect("unwrap in test")
        .finish()
        .expect("unwrap in test")
}

/// Build a tar.gz containing a symlink entry.
fn tar_gz_symlink(name: &str, target: &str) -> Vec<u8> {
    let gz = GzEncoder::new(Vec::new(), Compression::default());
    let mut ar = Builder::new(gz);
    let mut h = Header::new_gnu();
    h.set_path(name).expect("unwrap in test");
    h.set_size(0);
    h.set_entry_type(EntryType::Symlink);
    h.set_link_name(target).expect("unwrap in test");
    h.set_mode(0o777);
    h.set_cksum();
    ar.append(&h, &[][..]).expect("unwrap in test");
    ar.into_inner()
        .expect("unwrap in test")
        .finish()
        .expect("unwrap in test")
}

/// Build a raw tar.gz with a manually crafted header so we can embed filenames
/// that the tar crate's builder-level API would reject (e.g. `../escape.txt`).
///
/// Used specifically to test path traversal rejection because the safe tar
/// builder API validates paths at the Rust level before our code can reject them.
fn raw_tar_gz_with_traversal_filename(filename: &str) -> Vec<u8> {
    let mut header = [0u8; 512];
    let name = filename.as_bytes();
    let len = name.len().min(100);
    header[..len].copy_from_slice(&name[..len]);
    header[100..108].copy_from_slice(b"0000644\0");
    header[108..116].copy_from_slice(b"0000000\0");
    header[116..124].copy_from_slice(b"0000000\0");
    header[124..136].copy_from_slice(b"00000000000\0");
    header[136..148].copy_from_slice(b"00000000000\0");
    header[156] = b'0'; // regular file
    header[257..263].copy_from_slice(b"ustar ");
    header[263..265].copy_from_slice(b" \0");
    // Compute checksum with the field set to spaces.
    header[148..156].fill(b' ');
    let sum: u32 = header.iter().map(|&b| b as u32).sum();
    let cksum = format!("{sum:06o}\0 ");
    header[148..156].copy_from_slice(cksum.as_bytes());

    let mut tar_bytes = Vec::new();
    tar_bytes.extend_from_slice(&header);
    tar_bytes.extend_from_slice(&[0u8; 1024]); // two end-of-archive zero blocks

    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&tar_bytes).expect("unwrap in test");
    gz.finish().expect("unwrap in test")
}

// ---------------------------------------------------------------------------
// Regression 1: Zip Slip / path traversal (commits 8ea4f73, 2fc7036)
// ---------------------------------------------------------------------------

/// A tar entry with a leading `../` path component must be rejected.
///
/// This is the canonical Zip Slip attack: an attacker embeds `../evil.sh` in a
/// tar archive hoping to write a file outside the container rootfs.
///
/// Guards: commit `8ea4f73` — `validate_tar_entry_path` rejects `..` components.
// Invariant: 1 — Zip Slip / Path Traversal Prevention
#[test]
fn regression_zip_slip_dotdot_prefix_is_rejected() {
    let dest = TempDir::new().expect("unwrap in test");
    let tar_gz = raw_tar_gz_with_traversal_filename("../escape.txt");

    let err = extract_layer(&mut tar_gz.as_slice(), dest.path())
        .expect_err("path traversal must be rejected");

    assert!(
        err.to_string().contains("..") || err.to_string().contains("traversal"),
        "expected traversal error, got: {err}"
    );

    // Confirm nothing escaped the destination directory.
    let parent = dest.path().parent().expect("unwrap in test");
    assert!(
        !parent.join("escape.txt").exists(),
        "file must not have been written outside the container rootfs"
    );
}

/// A tar entry with `..` embedded in the middle of a path must also be rejected.
///
/// Example: `foo/../../etc/cron.d/evil` — looks like a sub-path but resolves above dest.
///
/// Guards: commit `8ea4f73`.
// Invariant: 1 — Zip Slip / Path Traversal Prevention
#[test]
fn regression_zip_slip_dotdot_in_middle_is_rejected() {
    let dest = TempDir::new().expect("unwrap in test");
    // Use the raw builder because the tar crate sanitises paths before our check.
    let tar_gz = raw_tar_gz_with_traversal_filename("foo/../../etc/passwd");

    let err = extract_layer(&mut tar_gz.as_slice(), dest.path())
        .expect_err("embedded .. must be rejected");

    assert!(
        err.to_string().contains("..") || err.to_string().contains("traversal"),
        "expected traversal error, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Regression 2: Device node extraction (commit 8ea4f73)
// ---------------------------------------------------------------------------

/// A tar entry of type `Block` (e.g. `/dev/sda`) must be rejected.
///
/// Extracting block device nodes allows a container image to ship files that
/// grant raw disk access to the host's storage devices.
///
/// Guards: commit `8ea4f73` — `EntryType::Block` / `EntryType::Char` are
/// rejected before `unpack_in` is called.
// Invariant: 2 — Device Node Extraction Rejection
#[test]
fn regression_block_device_node_is_rejected() {
    let dest = TempDir::new().expect("unwrap in test");
    let tar_gz = tar_gz_device_node("dev/sda", EntryType::Block);

    let err = extract_layer(&mut tar_gz.as_slice(), dest.path())
        .expect_err("block device node must be rejected");

    assert!(
        err.to_string().contains("device") || err.to_string().contains("DeviceNode"),
        "expected device rejection error, got: {err}"
    );

    assert!(
        !dest.path().join("dev/sda").exists(),
        "device node must not have been extracted"
    );
}

/// A tar entry of type `Char` (e.g. `/dev/null`) must also be rejected.
///
/// Character devices can be used to read random data from the host kernel or
/// access serial devices.
///
/// Guards: commit `8ea4f73`.
// Invariant: 2 — Device Node Extraction Rejection
#[test]
fn regression_char_device_node_is_rejected() {
    let dest = TempDir::new().expect("unwrap in test");
    let tar_gz = tar_gz_device_node("dev/null", EntryType::Char);

    let err = extract_layer(&mut tar_gz.as_slice(), dest.path())
        .expect_err("char device node must be rejected");

    assert!(
        err.to_string().contains("device") || err.to_string().contains("DeviceNode"),
        "expected device rejection error, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Regression 3: Absolute symlink with parent traversal (commit 2fc7036)
// ---------------------------------------------------------------------------

/// An absolute symlink whose relativised target still contains `..` must be
/// rejected.
///
/// Example: a symlink to `/../../etc/shadow`. After stripping the leading `/`
/// the target is `../../etc/shadow`, which contains `..` and could escape the
/// container rootfs.
///
/// Guards: commit `2fc7036` — `has_parent_dir_component` check on the
/// relativised target rejects these before the symlink is created.
// Invariant: 3 — Absolute Symlink Host Leakage Prevention
#[cfg(unix)]
#[test]
fn regression_absolute_symlink_with_traversal_is_rejected() {
    let dest = TempDir::new().expect("unwrap in test");
    // Target `/../../../etc/shadow` strips to `../../etc/shadow` — still has `..`.
    let tar_gz = tar_gz_symlink("evil_link", "/../../etc/shadow");

    let err = extract_layer(&mut tar_gz.as_slice(), dest.path())
        .expect_err("absolute symlink with traversal target must be rejected");

    assert!(
        err.to_string().contains("traversal") || err.to_string().contains(".."),
        "expected traversal error, got: {err}"
    );

    assert!(
        !dest.path().join("evil_link").exists(),
        "symlink must not have been created"
    );
}

/// An absolute symlink whose target resolves entirely within the container
/// rootfs must be *rewritten* to a relative path and accepted, not rejected.
///
/// Example: `bin/echo -> /bin/busybox` is valid — rewritten to `busybox`.
///
/// Guards: commit `2fc7036` — `relative_path()` computes the correct relative
/// target so the symlink works after `pivot_root`.
// Invariant: 3 — Absolute Symlink Host Leakage Prevention
#[cfg(unix)]
#[test]
fn regression_busybox_applet_symlink_is_rewritten_not_rejected() {
    let dest = TempDir::new().expect("unwrap in test");
    let tar_gz = tar_gz_symlink("bin/echo", "/bin/busybox");

    extract_layer(&mut tar_gz.as_slice(), dest.path())
        .expect("busybox applet symlink must be accepted and rewritten");

    let link = dest.path().join("bin/echo");
    assert!(
        link.symlink_metadata().is_ok(),
        "rewritten symlink must exist at bin/echo"
    );

    let target = std::fs::read_link(&link).expect("must be able to read the symlink target");
    assert!(
        !target.is_absolute(),
        "rewritten target must be relative, got: {target:?}"
    );
}

// ---------------------------------------------------------------------------
// Regression 4: Setuid/setgid bit stripping (commit 2fc7036)
// ---------------------------------------------------------------------------

/// A regular file extracted with setuid bits set must not retain those bits
/// after extraction.
///
/// Setuid binaries in a container image could escalate privilege to root if
/// not stripped. The extractor must clear bits 04000 (setuid), 02000 (setgid),
/// and 01000 (sticky) before writing to disk.
///
/// Guards: commit `2fc7036` — mode masking with `0o777` before `unpack_in`.
///
/// Note: the tar crate applies the mode from the header when extracting. The
/// production code calls `entry.header_mut().set_mode(safe_mode)` before
/// `unpack_in`. This test verifies the end-to-end behaviour: a file shipped
/// with mode `04755` (setuid + rwxr-xr-x) must land with mode `0755`.
// Invariant: 4 — Setuid / Setgid Bit Stripping
#[cfg(unix)]
#[test]
fn regression_setuid_bits_stripped_on_extraction() {
    use std::os::unix::fs::PermissionsExt;

    let dest = TempDir::new().expect("unwrap in test");
    // 04755 = setuid + rwxr-xr-x
    let tar_gz = tar_gz_regular_file("usr/bin/setuid_binary", b"#!/bin/sh", 0o4755);

    extract_layer(&mut tar_gz.as_slice(), dest.path())
        .expect("setuid file must be extracted without error");

    let path = dest.path().join("usr/bin/setuid_binary");
    assert!(path.exists(), "file must have been extracted");

    let mode = std::fs::metadata(&path)
        .expect("must be able to stat extracted file")
        .permissions()
        .mode();

    // The setuid bit (04000) must be absent.
    assert_eq!(
        mode & 0o4000,
        0,
        "setuid bit must be stripped; got mode {mode:o}"
    );
    // The setgid bit (02000) must also be absent.
    assert_eq!(
        mode & 0o2000,
        0,
        "setgid bit must be stripped; got mode {mode:o}"
    );
}

// ---------------------------------------------------------------------------
// Regression 5: FD-leak prevention — close_extra_fds (process.rs)
// ---------------------------------------------------------------------------

/// Verify that `close_extra_fds` in `process.rs` closes FDs above stderr.
///
/// This is a source-level invariant test: the function must exist and use
/// `close_range(3, ...)` or `/proc/self/fd` fallback. We verify the source
/// contains the expected syscall invocation so that refactors that remove or
/// weaken FD closure are caught.
///
/// The actual FD closure is Linux-only (requires `/proc/self/fd` or
/// `close_range` syscall) so we test the contract via source inspection.
// Invariant: 5 — FD-Leak Prevention in Child Init
#[test]
fn regression_close_extra_fds_uses_close_range_syscall() {
    let source = include_str!("../src/container/process.rs");

    // Must use close_range starting from FD 3 (preserve stdin/stdout/stderr).
    assert!(
        source.contains("SYS_close_range"),
        "close_extra_fds must use close_range syscall as fast path"
    );
    assert!(
        source.contains("FIRST_NON_STDIO_FD: u32 = 3"),
        "close_range must start from FD 3 (preserving stdin/stdout/stderr)"
    );

    // Must have /proc/self/fd fallback for older kernels.
    assert!(
        source.contains("/proc/self/fd"),
        "close_extra_fds must fall back to /proc/self/fd scan"
    );

    // Must filter out FDs <= 2.
    assert!(
        source.contains("fd > 2"),
        "fallback path must skip stdin/stdout/stderr (fd > 2 filter)"
    );
}

// ---------------------------------------------------------------------------
// Regression 6: Environment isolation — execve not execvp (process.rs)
// ---------------------------------------------------------------------------

/// Verify that `child_init` uses `execve` (explicit envp) instead of `execvp`
/// (inherits host environment).
///
/// `execvp` would leak the daemon's entire environment into every container,
/// exposing secrets, API keys, and host configuration. `execve` takes an
/// explicit `envp` parameter built from `config.env`, ensuring only declared
/// variables are visible inside the container.
///
/// This is a critical security invariant: if someone changes the exec call
/// to `execvp`, this test must fail.
// Invariant: 6 — Environment Isolation (execve not execvp)
#[test]
fn regression_child_init_uses_execve_not_execvp() {
    let source = include_str!("../src/container/process.rs");

    // child_init must call execve (with explicit envp).
    assert!(
        source.contains("execve(&cmd, &argv, &envp)"),
        "child_init must use execve with explicit envp, not execvp"
    );

    // The source must NOT contain execvp calls (which inherit host env).
    // We check for the nix crate's execvp function specifically.
    let has_execvp_call = source.lines().any(|line| {
        let trimmed = line.trim();
        // Skip comments and string literals
        !trimmed.starts_with("//")
                && !trimmed.starts_with("///")
                && !trimmed.starts_with("*")
                && trimmed.contains("execvp(")
                // Exclude references in comments about what NOT to do
                && !trimmed.contains("not")
                && !trimmed.contains("NOT")
                && !trimmed.contains("Do not")
    });
    assert!(
        !has_execvp_call,
        "child_init must not use execvp — it leaks the host environment into containers"
    );
}

/// Verify that the envp vector in child_init is built from `config.env`,
/// not from `std::env::vars()` or any other host-environment source.
// Invariant: 6 — Environment Isolation (execve not execvp)
#[test]
fn regression_envp_built_from_config_env_only() {
    let source = include_str!("../src/container/process.rs");

    // The envp must be constructed from config.env.
    assert!(
        source.contains("config.env"),
        "envp must be built from config.env (container-declared variables only)"
    );

    // Must NOT read from the host environment.
    assert!(
        !source.contains("std::env::vars()"),
        "child_init must not read host environment via std::env::vars()"
    );
}

// ---------------------------------------------------------------------------
// Regression 7: Named pipe / FIFO rejection in tar extraction
// ---------------------------------------------------------------------------

/// A tar entry of type `Fifo` (named pipe) must be handled safely.
///
/// Named pipes in a container image could be used for denial-of-service
/// by blocking reads during extraction. While the current implementation
/// allows FIFOs through `unpack_in`, this test documents the behaviour
/// and ensures no regression if FIFO rejection is added later.
// Invariant: 9 — FIFO / Named Pipe Non-Crash Guarantee
#[test]
fn regression_fifo_entry_does_not_crash() {
    let dest = TempDir::new().expect("failed to create temp dir");
    let gz = GzEncoder::new(Vec::new(), Compression::default());
    let mut ar = Builder::new(gz);
    let mut h = Header::new_gnu();
    h.set_path("tmp/fifo").expect("set_path");
    h.set_size(0);
    h.set_entry_type(EntryType::Fifo);
    h.set_mode(0o644);
    h.set_cksum();
    ar.append(&h, &[][..]).expect("append");
    let tar_gz = ar.into_inner().expect("inner").finish().expect("finish");

    // FIFOs are not explicitly rejected like device nodes, but extraction
    // must not panic or corrupt state. The result may be Ok or Err depending
    // on platform, but it must not panic.
    let _result = extract_layer(&mut tar_gz.as_slice(), dest.path());
}

// ---------------------------------------------------------------------------
// Regression 8: Root dot entry skip (tar archive markers)
// ---------------------------------------------------------------------------

/// The tar root marker entries "." and "./" must be silently skipped.
///
/// These entries appear in many OCI layer tarballs as the archive root
/// directory. Without the skip, `validate_tar_entry_path` would reject
/// them because `Path::join("./")` normalises away the CurDir component,
/// causing a confusing false-positive path-escape error.
// Invariant: 8 — Tar Root Entry Skip
#[test]
fn regression_root_dot_entries_are_silently_skipped() {
    let dest = TempDir::new().expect("failed to create temp dir");

    // "." entry
    let tar_gz_dot = tar_gz_regular_file(".", b"", 0o644);
    extract_layer(&mut tar_gz_dot.as_slice(), dest.path())
        .expect("'.' root entry must be silently skipped, not rejected");

    // "./" entry
    let tar_gz_dot_slash = tar_gz_regular_file("./", b"", 0o644);
    extract_layer(&mut tar_gz_dot_slash.as_slice(), dest.path())
        .expect("'./' root entry must be silently skipped, not rejected");
}

// ---------------------------------------------------------------------------
// Mutation audit: Invariant 10 — Request Size Limit
// ---------------------------------------------------------------------------

/// Verify that MAX_REQUEST_SIZE is defined and enforced in the daemon server.
///
/// Removing or raising this constant beyond 1 MB would allow malicious clients
/// to exhaust daemon memory with oversized JSON payloads.
///
/// Guard location: `crates/minibox/src/daemon/server.rs` — `MAX_REQUEST_SIZE`.
// Invariant: 10 — Request Size Limit
#[test]
fn mutation_audit_request_size_limit_exists() {
    let source = include_str!("../src/daemon/server.rs");

    // The constant must exist.
    assert!(
        source.contains("MAX_REQUEST_SIZE"),
        "MAX_REQUEST_SIZE constant must be defined in server.rs"
    );

    // The constant must be used in bounded_read_line call.
    assert!(
        source.contains("bounded_read_line") && source.contains("MAX_REQUEST_SIZE"),
        "MAX_REQUEST_SIZE must be passed to bounded_read_line"
    );

    // The limit must be 1 MB (1_048_576 bytes). Detect if someone raises it.
    assert!(
        source.contains("1024 * 1024") || source.contains("1_048_576"),
        "MAX_REQUEST_SIZE must be 1 MB (1024 * 1024 or 1_048_576)"
    );
}

// ---------------------------------------------------------------------------
// Mutation audit: Invariant 11 — Image Pull Resource Limits
// ---------------------------------------------------------------------------

/// Verify that image pull size limit constants exist and are enforced in the
/// registry client.
///
/// Removing these constants would allow unbounded manifest/layer downloads,
/// enabling DoS via oversized image pulls.
///
/// Guard location: `crates/minibox-core/src/image/registry.rs`.
// Invariant: 11 — Image Pull Resource Limits
#[test]
fn mutation_audit_image_pull_size_limits_exist() {
    let source = include_str!("../../minibox-core/src/image/registry.rs");

    // All three constants must exist.
    assert!(
        source.contains("MAX_MANIFEST_SIZE"),
        "MAX_MANIFEST_SIZE constant must be defined in registry.rs"
    );
    assert!(
        source.contains("MAX_LAYER_SIZE"),
        "MAX_LAYER_SIZE constant must be defined in registry.rs"
    );
    assert!(
        source.contains("MAX_TOTAL_IMAGE_SIZE"),
        "MAX_TOTAL_IMAGE_SIZE constant must be defined in registry.rs"
    );

    // LimitedStream must exist as the enforcement mechanism.
    assert!(
        source.contains("LimitedStream"),
        "LimitedStream streaming limiter must be present in registry.rs"
    );
}

// ---------------------------------------------------------------------------
// Mutation audit: Invariant 12 — Execution Manifest Integrity
// ---------------------------------------------------------------------------

/// Verify that execution manifest env var hashing and seal logic exist.
///
/// Removing the SHA-256 hashing of env values would expose secrets in
/// plaintext in the manifest file. Removing seal() would break workload
/// digest computation.
///
/// Guard location: `crates/minibox-core/src/domain/execution_manifest.rs`.
// Invariant: 12 — Execution Manifest Integrity
#[test]
fn mutation_audit_execution_manifest_env_hashing() {
    let source = include_str!("../../minibox-core/src/domain/execution_manifest.rs");

    // Env values must be hashed with SHA-256, never stored as plaintext.
    assert!(
        source.contains("Sha256") || source.contains("sha2"),
        "execution manifest must use SHA-256 for env value hashing"
    );

    // The seal() method must exist for workload digest computation.
    assert!(
        source.contains("fn seal("),
        "ExecutionManifest must have a seal() method"
    );

    // The digest must exclude volatile fields.
    assert!(
        source.contains("workload_digest") && source.contains("created_at"),
        "manifest must reference workload_digest and created_at fields"
    );
}

// ---------------------------------------------------------------------------
// Mutation audit: Invariant 1 — Zip Slip guard exists in source
// ---------------------------------------------------------------------------

/// Verify that `validate_tar_entry_path` exists and checks for `ParentDir`
/// components in `layer.rs`.
///
/// The behavioral tests (regression_zip_slip_*) confirm the guard works.
/// This test confirms the guard mechanism itself is present in source,
/// catching refactors that might remove the function or its core check.
///
/// Guard location: `crates/minibox-core/src/image/layer.rs`.
// Invariant: 1 — Zip Slip / Path Traversal Prevention
#[test]
fn mutation_audit_zip_slip_guard_exists() {
    let source = include_str!("../../minibox-core/src/image/layer.rs");

    // The validation function must exist.
    assert!(
        source.contains("fn validate_tar_entry_path"),
        "validate_tar_entry_path function must exist in layer.rs"
    );

    // It must check for ParentDir components.
    assert!(
        source.contains("ParentDir"),
        "validate_tar_entry_path must check for ParentDir (dotdot) components"
    );

    // It must be called during extraction.
    assert!(
        source.contains("validate_tar_entry_path(&entry_path"),
        "validate_tar_entry_path must be called during layer extraction"
    );
}

// ---------------------------------------------------------------------------
// Mutation audit: Invariant 2 — Device node rejection guard exists in source
// ---------------------------------------------------------------------------

/// Verify that device node rejection logic exists in `layer.rs`.
///
/// Guard location: `crates/minibox-core/src/image/layer.rs`.
// Invariant: 2 — Device Node Extraction Rejection
#[test]
fn mutation_audit_device_node_rejection_exists() {
    let source = include_str!("../../minibox-core/src/image/layer.rs");

    // Must reference Block and Char entry types for rejection.
    assert!(
        source.contains("Block") && source.contains("Char"),
        "layer.rs must check for Block and Char entry types"
    );

    // Must reference the DeviceNodeRejected error.
    assert!(
        source.contains("DeviceNodeRejected"),
        "layer.rs must return DeviceNodeRejected error for device nodes"
    );
}

// ---------------------------------------------------------------------------
// Mutation audit: Invariant 3 — Absolute symlink rewrite guard exists
// ---------------------------------------------------------------------------

/// Verify that absolute symlink rewriting and traversal rejection exist
/// in `layer.rs`.
///
/// Guard location: `crates/minibox-core/src/image/layer.rs`.
// Invariant: 3 — Absolute Symlink Host Leakage Prevention
#[test]
fn mutation_audit_symlink_rewrite_guard_exists() {
    let source = include_str!("../../minibox-core/src/image/layer.rs");

    // Must have the relative_path function for rewriting absolute targets.
    assert!(
        source.contains("fn relative_path"),
        "relative_path function must exist in layer.rs for symlink rewriting"
    );

    // Must check for parent dir components in rewritten targets.
    assert!(
        source.contains("has_parent_dir_component"),
        "has_parent_dir_component must be used to reject traversal in rewritten symlinks"
    );
}

// ---------------------------------------------------------------------------
// Mutation audit: Invariant 4 — Setuid bit stripping guard exists
// ---------------------------------------------------------------------------

/// Verify that setuid/setgid bit stripping logic exists in `layer.rs`.
///
/// The mode mask `0o777` strips bits above the permission triad (setuid,
/// setgid, sticky). Removing this mask would allow privilege escalation
/// via setuid binaries in OCI layers.
///
/// Guard location: `crates/minibox-core/src/image/layer.rs`.
// Invariant: 4 — Setuid / Setgid Bit Stripping
#[test]
fn mutation_audit_setuid_strip_guard_exists() {
    let source = include_str!("../../minibox-core/src/image/layer.rs");

    // Must contain the 0o777 mode mask that strips setuid/setgid/sticky bits.
    assert!(
        source.contains("0o777"),
        "layer.rs must contain 0o777 mode mask for setuid stripping"
    );

    // Must call set_mode on the header before unpacking.
    assert!(
        source.contains("set_mode"),
        "layer.rs must call set_mode to apply the stripped mode"
    );
}

// ---------------------------------------------------------------------------
// Mutation audit: Invariant 7 — SO_PEERCRED guard called in handler
// ---------------------------------------------------------------------------

/// Verify that `is_authorized` is called in the connection handler in
/// `server.rs`, not just defined.
///
/// The behavioral tests in `daemon_security_regression.rs` verify the
/// function's logic. This test verifies the function is actually invoked
/// in the request processing path.
///
/// Guard location: `crates/minibox/src/daemon/server.rs`.
// Invariant: 7 — SO_PEERCRED Unix Socket Authentication
#[test]
fn mutation_audit_peercred_guard_called_in_handler() {
    let source = include_str!("../src/daemon/server.rs");

    // is_authorized must be called (not just defined).
    let call_count = source.matches("is_authorized(").count();
    // At least 2: 1 definition + 1 call site in handler.
    assert!(
        call_count >= 2,
        "is_authorized must be called in the connection handler, \
         found {call_count} occurrences (need >= 2: definition + call)"
    );

    // The handler must reject unauthorized connections.
    assert!(
        source.contains("!is_authorized("),
        "connection handler must check !is_authorized to reject unauthorized clients"
    );
}
