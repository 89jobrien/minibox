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
    h.set_path(name).unwrap();
    h.set_size(content.len() as u64);
    h.set_entry_type(EntryType::Regular);
    h.set_mode(mode);
    h.set_cksum();
    ar.append(&h, content).unwrap();
    ar.into_inner().unwrap().finish().unwrap()
}

/// Build a tar.gz containing a device node entry.
fn tar_gz_device_node(name: &str, kind: EntryType) -> Vec<u8> {
    let gz = GzEncoder::new(Vec::new(), Compression::default());
    let mut ar = Builder::new(gz);
    let mut h = Header::new_gnu();
    h.set_path(name).unwrap();
    h.set_size(0);
    h.set_entry_type(kind);
    h.set_mode(0o644);
    h.set_cksum();
    ar.append(&h, &[][..]).unwrap();
    ar.into_inner().unwrap().finish().unwrap()
}

/// Build a tar.gz containing a symlink entry.
fn tar_gz_symlink(name: &str, target: &str) -> Vec<u8> {
    let gz = GzEncoder::new(Vec::new(), Compression::default());
    let mut ar = Builder::new(gz);
    let mut h = Header::new_gnu();
    h.set_path(name).unwrap();
    h.set_size(0);
    h.set_entry_type(EntryType::Symlink);
    h.set_link_name(target).unwrap();
    h.set_mode(0o777);
    h.set_cksum();
    ar.append(&h, &[][..]).unwrap();
    ar.into_inner().unwrap().finish().unwrap()
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
    gz.write_all(&tar_bytes).unwrap();
    gz.finish().unwrap()
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
#[test]
fn regression_zip_slip_dotdot_prefix_is_rejected() {
    let dest = TempDir::new().unwrap();
    let tar_gz = raw_tar_gz_with_traversal_filename("../escape.txt");

    let err = extract_layer(&mut tar_gz.as_slice(), dest.path())
        .expect_err("path traversal must be rejected");

    assert!(
        err.to_string().contains("..") || err.to_string().contains("traversal"),
        "expected traversal error, got: {err}"
    );

    // Confirm nothing escaped the destination directory.
    let parent = dest.path().parent().unwrap();
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
#[test]
fn regression_zip_slip_dotdot_in_middle_is_rejected() {
    let dest = TempDir::new().unwrap();
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
#[test]
fn regression_block_device_node_is_rejected() {
    let dest = TempDir::new().unwrap();
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
#[test]
fn regression_char_device_node_is_rejected() {
    let dest = TempDir::new().unwrap();
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
#[cfg(unix)]
#[test]
fn regression_absolute_symlink_with_traversal_is_rejected() {
    let dest = TempDir::new().unwrap();
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
#[cfg(unix)]
#[test]
fn regression_busybox_applet_symlink_is_rewritten_not_rejected() {
    let dest = TempDir::new().unwrap();
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
#[cfg(unix)]
#[test]
fn regression_setuid_bits_stripped_on_extraction() {
    use std::os::unix::fs::PermissionsExt;

    let dest = TempDir::new().unwrap();
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
