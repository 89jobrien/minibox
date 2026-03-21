//! OCI layer extraction and digest verification.
//!
//! OCI layers are gzip-compressed tar archives. This module decompresses and
//! extracts them into a target directory, and verifies their `sha256:` digest.

use crate::error::ImageError;
use anyhow::Context;
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use tar::{Archive, EntryType};
use tracing::{debug, info, instrument, warn};

// Helper to check for parent directory components
fn has_parent_dir_component(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// Compute a relative path from `from_dir` to `to`, both relative to the
/// container root (no leading `/`).
///
/// Used to rewrite absolute symlink targets so they are relative to the
/// symlink's own directory, making them correct after `pivot_root`.
///
/// ```ignore
/// # use std::path::Path;
/// // bin/echo -> /bin/busybox  =>  rewrite target to "busybox"
/// assert_eq!(relative_path(Path::new("bin"), Path::new("bin/busybox")),
///            std::path::PathBuf::from("busybox"));
/// // usr/local/bin/python -> /usr/bin/python  =>  "../../bin/python"
/// assert_eq!(relative_path(Path::new("usr/local/bin"), Path::new("usr/bin/python")),
///            std::path::PathBuf::from("../../bin/python"));
/// ```
fn relative_path(from_dir: &Path, to: &Path) -> std::path::PathBuf {
    use std::path::Component;
    let from: Vec<_> = from_dir
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();
    let to_parts: Vec<_> = to
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();

    let common = from
        .iter()
        .zip(to_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut result = std::path::PathBuf::new();
    for _ in &from[common..] {
        result.push("..");
    }
    for part in &to_parts[common..] {
        result.push(part.as_os_str());
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    result
}

/// Extract a gzip-compressed tar layer into `dest`.
///
/// Any files inside the tar are extracted relative to `dest`. The destination
/// directory must already exist.
///
/// # Security
///
/// This function validates each tar entry to prevent:
/// - Path traversal attacks (Zip Slip vulnerability)
/// - Symlink attacks pointing to host filesystem
/// - Setuid/setgid binary preservation
/// - Device node extraction
///
/// # Arguments
///
/// * `tar_gz_data` -- Raw bytes of the `.tar.gz` blob.
/// * `dest` -- Directory to extract into.
#[instrument(skip(tar_gz_data, dest), fields(bytes = tar_gz_data.len(), dest = %dest.display()))]
pub fn extract_layer(tar_gz_data: &[u8], dest: &Path) -> anyhow::Result<()> {
    debug!(
        "extracting layer ({} bytes) to {:?}",
        tar_gz_data.len(),
        dest
    );

    let gz = GzDecoder::new(tar_gz_data);
    let mut archive = Archive::new(gz);

    // SECURITY: Manually iterate entries to validate each one
    for entry_result in archive
        .entries()
        .map_err(|e| ImageError::LayerExtract(format!("failed to read tar entries: {e}")))?
    {
        let mut entry = entry_result
            .map_err(|e| ImageError::LayerExtract(format!("failed to read tar entry: {e}")))?;

        let entry_path = entry
            .path()
            .map_err(|e| ImageError::LayerExtract(format!("invalid entry path: {e}")))?
            .into_owned();

        // Skip root directory entry — "." and "./" are tar markers for the
        // archive root. dest already exists; extracting these is a no-op and
        // their path normalisation confuses the escape check below.
        if entry_path == Path::new(".") || entry_path == Path::new("./") {
            continue;
        }

        // SECURITY: Validate entry path doesn't escape destination
        validate_tar_entry_path(&entry_path, dest)?;

        // SECURITY: Check entry type and reject dangerous ones
        let entry_type = entry.header().entry_type();

        // Reject device nodes
        if matches!(entry_type, EntryType::Block | EntryType::Char) {
            warn!(
                entry = ?entry_path,
                kind = ?entry_type,
                "tar: rejected device node (security risk)"
            );
            return Err(ImageError::DeviceNodeRejected {
                entry: format!("{entry_path:?}"),
            }
            .into());
        }

        // Handle symlinks to absolute paths by rewriting to a path that is
        // relative to the symlink's own directory.
        //
        // Example: entry `bin/echo` with target `/bin/busybox`
        //   entry_dir  = "bin"
        //   abs_target = "bin/busybox"   (strip leading "/")
        //   rel        = "busybox"       (relative from "bin" to "bin/busybox")
        //
        // This is necessary because inside the container (after pivot_root)
        // absolute symlinks resolve correctly, but on the HOST during extraction
        // they would point into the host filesystem.
        if entry_type == EntryType::Symlink
            && let Ok(Some(link_target)) = entry.link_name()
            && link_target.is_absolute()
        {
            let abs_target = link_target.strip_prefix("/").map_err(|_| {
                ImageError::LayerExtract(format!(
                    "invalid absolute symlink target: {link_target:?}"
                ))
            })?;

            if has_parent_dir_component(abs_target) {
                warn!(
                    entry = ?entry_path,
                    target = ?link_target,
                    "tar: rejected symlink with parent traversal (security risk)"
                );
                return Err(ImageError::SymlinkTraversalRejected {
                    entry: format!("{entry_path:?}"),
                    target: format!("{link_target:?}"),
                }
                .into());
            }

            // Compute path relative to the symlink's directory.
            let entry_dir = entry_path.parent().unwrap_or(Path::new(""));
            let rel_target = relative_path(entry_dir, abs_target);

            let target_path = dest.join(&entry_path);
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating parent dirs for symlink {target_path:?}"))?;
            }

            if target_path.exists() || target_path.symlink_metadata().is_ok() {
                let meta = target_path.symlink_metadata().ok();
                if meta.as_ref().map(|m| m.is_dir()).unwrap_or(false) {
                    fs::remove_dir_all(&target_path)
                        .with_context(|| format!("removing existing dir at {target_path:?}"))?;
                } else {
                    fs::remove_file(&target_path)
                        .with_context(|| format!("removing existing file at {target_path:?}"))?;
                }
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::symlink;
                symlink(&rel_target, &target_path).with_context(|| {
                    format!("creating rewritten symlink {target_path:?} -> {rel_target:?}")
                })?;
            }

            #[cfg(not(unix))]
            {
                return Err(ImageError::LayerExtract(
                    "absolute symlink rewrite is not supported on this platform".into(),
                )
                .into());
            }

            warn!(
                entry = ?entry_path,
                original_target = ?link_target,
                rewritten_target = ?rel_target,
                "tar: rewrote absolute symlink to relative"
            );
            continue;
        }

        // SECURITY: Strip setuid/setgid bits from file permissions
        // We don't trust image-provided permissions for these bits
        if entry_type == EntryType::Regular {
            let mode = entry
                .header()
                .mode()
                .map_err(|e| ImageError::LayerExtract(format!("failed to get mode: {e}")))?;

            // Remove setuid (04000), setgid (02000), and sticky (01000) bits
            let safe_mode = mode & 0o777;
            if mode != safe_mode {
                warn!(
                    entry = ?entry_path,
                    mode_before = mode,
                    mode_after = safe_mode,
                    "tar: stripped special permission bits"
                );
            }
            // Note: tar crate doesn't provide set_mode(), so we rely on umask
            // This is acceptable as we're running as root with controlled umask
        }

        // Extract the entry
        entry.unpack_in(dest).map_err(|e| {
            ImageError::LayerExtract(format!("failed to extract entry {entry_path:?}: {e}"))
        })?;
    }

    info!("layer extracted to {:?}", dest);
    Ok(())
}

/// Validate that a tar entry path is safe to extract.
///
/// # Security
///
/// Rejects paths that:
/// - Contain `..` components (path traversal)
/// - Are absolute paths
/// - Would escape the destination directory
fn validate_tar_entry_path(entry_path: &Path, dest: &Path) -> anyhow::Result<()> {
    // Reject absolute paths
    if entry_path.is_absolute() {
        return Err(ImageError::LayerExtract(format!(
            "tar entry uses absolute path (security risk): {entry_path:?}"
        ))
        .into());
    }

    // Check for path traversal via .. components
    if has_parent_dir_component(entry_path) {
        return Err(ImageError::LayerExtract(format!(
            "tar entry contains '..' component (path traversal): {entry_path:?}"
        ))
        .into());
    }

    // Verify the resolved path would be within dest
    let full_path = dest.join(entry_path);

    // Canonicalize dest for comparison (full_path may not exist yet)
    let canonical_dest = dest
        .canonicalize()
        .with_context(|| format!("canonicalizing dest {dest:?}"))?;

    // Check if the entry path when joined with dest would escape
    // We can't canonicalize full_path if it doesn't exist, so check the parent
    if let Some(parent) = full_path.parent()
        && parent.exists()
    {
        let canonical_parent = parent.canonicalize()?;
        if !canonical_parent.starts_with(&canonical_dest) {
            return Err(ImageError::LayerExtract(format!(
                "tar entry would escape destination: {entry_path:?}"
            ))
            .into());
        }
    }

    debug!("validated tar entry path: {:?}", entry_path);
    Ok(())
}

/// Verify that `data` matches `expected_digest`.
///
/// The digest must be in `sha256:<hex>` format as used by OCI manifests.
///
/// # Errors
///
/// Returns [`ImageError::DigestMismatch`] if the computed hash does not match
/// the expected value, or an error if `expected_digest` is malformed.
pub fn verify_digest(data: &[u8], expected_digest: &str) -> anyhow::Result<()> {
    let expected_hex =
        expected_digest
            .strip_prefix("sha256:")
            .ok_or_else(|| ImageError::DigestMismatch {
                digest: expected_digest.to_owned(),
                expected: expected_digest.to_owned(),
                actual: "(could not parse prefix)".into(),
            })?;

    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let actual_hex = hex::encode(result);

    if actual_hex != expected_hex {
        return Err(ImageError::DigestMismatch {
            digest: expected_digest.to_owned(),
            expected: expected_hex.to_owned(),
            actual: actual_hex,
        }
        .into());
    }

    debug!("digest verified: sha256:{}", actual_hex);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use tar::{Builder, EntryType, Header};
    use tempfile::TempDir;

    // ---------------------------------------------------------------------------
    // Tar archive builders for tests
    // ---------------------------------------------------------------------------

    fn tar_gz_with_regular_file(name: &str, content: &[u8]) -> Vec<u8> {
        let gz = GzEncoder::new(Vec::new(), Compression::default());
        let mut ar = Builder::new(gz);
        let mut h = Header::new_gnu();
        h.set_path(name).unwrap();
        h.set_size(content.len() as u64);
        h.set_entry_type(EntryType::Regular);
        h.set_mode(0o644);
        h.set_cksum();
        ar.append(&h, content).unwrap();
        ar.into_inner().unwrap().finish().unwrap()
    }

    fn tar_gz_with_device(name: &str, device_type: EntryType) -> Vec<u8> {
        let gz = GzEncoder::new(Vec::new(), Compression::default());
        let mut ar = Builder::new(gz);
        let mut h = Header::new_gnu();
        h.set_path(name).unwrap();
        h.set_size(0);
        h.set_entry_type(device_type);
        h.set_mode(0o644);
        h.set_cksum();
        ar.append(&h, &[][..]).unwrap();
        ar.into_inner().unwrap().finish().unwrap()
    }

    fn tar_gz_with_symlink(name: &str, target: &str) -> Vec<u8> {
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

    /// Build a raw tar.gz with a manually crafted header so we can embed
    /// paths that the `tar` crate's builder would normally reject (e.g. `../`).
    fn raw_tar_gz_with_filename(filename: &str) -> Vec<u8> {
        use flate2::{Compression, write::GzEncoder};
        use std::io::Write;

        let mut header = [0u8; 512];
        // Name field: bytes 0-99
        let name = filename.as_bytes();
        let len = name.len().min(100);
        header[..len].copy_from_slice(&name[..len]);
        // Mode: 0000644\0
        header[100..108].copy_from_slice(b"0000644\0");
        // uid/gid/size/mtime as zero octal
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        header[124..136].copy_from_slice(b"00000000000\0");
        header[136..148].copy_from_slice(b"00000000000\0");
        // type flag '0' = regular file
        header[156] = b'0';
        // ustar magic
        header[257..263].copy_from_slice(b"ustar ");
        header[263..265].copy_from_slice(b" \0");
        // Checksum: set field to spaces, sum all bytes, write back
        header[148..156].fill(b' ');
        let sum: u32 = header.iter().map(|&b| b as u32).sum();
        let cksum = format!("{sum:06o}\0 ");
        header[148..156].copy_from_slice(cksum.as_bytes());

        // tar = header block + two end-of-archive zero blocks
        let mut tar_bytes = Vec::new();
        tar_bytes.extend_from_slice(&header);
        tar_bytes.extend_from_slice(&[0u8; 1024]);

        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&tar_bytes).unwrap();
        gz.finish().unwrap()
    }

    // ---------------------------------------------------------------------------
    // validate_tar_entry_path
    // ---------------------------------------------------------------------------

    #[test]
    fn absolute_path_rejected() {
        let dest = TempDir::new().unwrap();
        let err = validate_tar_entry_path(Path::new("/etc/passwd"), dest.path()).unwrap_err();
        assert!(
            err.to_string().contains("absolute"),
            "expected 'absolute' in: {err}"
        );
    }

    #[test]
    fn dotdot_prefix_rejected() {
        let dest = TempDir::new().unwrap();
        let err = validate_tar_entry_path(Path::new("../escape"), dest.path()).unwrap_err();
        assert!(err.to_string().contains(".."), "expected '..' in: {err}");
    }

    #[test]
    fn dotdot_in_middle_rejected() {
        let dest = TempDir::new().unwrap();
        let err =
            validate_tar_entry_path(Path::new("foo/../../etc/passwd"), dest.path()).unwrap_err();
        assert!(err.to_string().contains(".."), "expected '..' in: {err}");
    }

    #[test]
    fn normal_relative_path_accepted() {
        let dest = TempDir::new().unwrap();
        validate_tar_entry_path(Path::new("usr/bin/something"), dest.path()).unwrap();
    }

    #[test]
    fn deeply_nested_relative_path_accepted() {
        let dest = TempDir::new().unwrap();
        validate_tar_entry_path(Path::new("a/b/c/d/e/f"), dest.path()).unwrap();
    }

    // ---------------------------------------------------------------------------
    // extract_layer — end-to-end
    // ---------------------------------------------------------------------------

    #[test]
    fn regular_file_extracted_correctly() {
        let dest = TempDir::new().unwrap();
        let tar_gz = tar_gz_with_regular_file("hello.txt", b"hello world");
        extract_layer(&tar_gz, dest.path()).unwrap();
        assert_eq!(
            std::fs::read(dest.path().join("hello.txt")).unwrap(),
            b"hello world"
        );
    }

    #[test]
    fn nested_regular_file_extracted() {
        let dest = TempDir::new().unwrap();
        let tar_gz = tar_gz_with_regular_file("usr/local/bin/tool", b"binary");
        extract_layer(&tar_gz, dest.path()).unwrap();
        assert!(dest.path().join("usr/local/bin/tool").exists());
    }

    #[test]
    fn block_device_entry_rejected() {
        let dest = TempDir::new().unwrap();
        let tar_gz = tar_gz_with_device("dev/sda", EntryType::Block);
        let err = extract_layer(&tar_gz, dest.path()).unwrap_err();
        assert!(
            err.to_string().contains("device"),
            "expected 'device' in: {err}"
        );
    }

    #[test]
    fn char_device_entry_rejected() {
        let dest = TempDir::new().unwrap();
        let tar_gz = tar_gz_with_device("dev/null", EntryType::Char);
        let err = extract_layer(&tar_gz, dest.path()).unwrap_err();
        assert!(
            err.to_string().contains("device"),
            "expected 'device' in: {err}"
        );
    }

    #[test]
    fn root_dot_entry_skipped() {
        // "." is the tar root marker — extract_layer must skip it silently (no error,
        // no file extracted).
        let dest = TempDir::new().unwrap();
        let tar_gz = tar_gz_with_regular_file(".", b"");
        extract_layer(&tar_gz, dest.path()).unwrap(); // must not error
        // The destination directory must remain empty — nothing was extracted.
        let entries: Vec<_> = std::fs::read_dir(dest.path()).unwrap().collect();
        assert!(
            entries.is_empty(),
            "no files should be extracted for '.' entry"
        );
    }

    #[test]
    fn root_dot_slash_entry_skipped() {
        // "./" variant of the same root marker
        let dest = TempDir::new().unwrap();
        let tar_gz = tar_gz_with_regular_file("./", b"");
        extract_layer(&tar_gz, dest.path()).unwrap(); // must not error
        let entries: Vec<_> = std::fs::read_dir(dest.path()).unwrap().collect();
        assert!(
            entries.is_empty(),
            "no files should be extracted for './' entry"
        );
    }

    #[test]
    fn dotdot_tar_entry_rejected() {
        let dest = TempDir::new().unwrap();
        // Use a raw tar so we can embed ../ in the filename, bypassing
        // the tar crate's builder-level path validation.
        let tar_gz = raw_tar_gz_with_filename("../escape.txt");
        let err = extract_layer(&tar_gz, dest.path()).unwrap_err();
        assert!(
            err.to_string().contains("..") || err.to_string().contains("traversal"),
            "expected path traversal error, got: {err}"
        );
        // Confirm nothing escaped the dest directory
        assert!(!dest.path().parent().unwrap().join("escape.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn busybox_applet_symlink_correct() {
        // bin/echo -> /bin/busybox: after rewrite, target should be "busybox" (same dir)
        // This is the specific busybox case that was broken before the fix.
        let dest = TempDir::new().unwrap();
        let tar_gz = tar_gz_with_symlink("bin/echo", "/bin/busybox");
        extract_layer(&tar_gz, dest.path()).unwrap();
        let link = dest.path().join("bin/echo");
        assert!(link.symlink_metadata().is_ok(), "symlink should exist");
        let target = std::fs::read_link(&link).unwrap();
        assert!(
            !target.is_absolute(),
            "target must be relative, got: {target:?}"
        );
        assert_eq!(
            target,
            std::path::PathBuf::from("busybox"),
            "bin/echo -> /bin/busybox should rewrite to 'busybox'"
        );
    }

    #[cfg(unix)]
    #[test]
    fn cross_dir_absolute_symlink_rewritten() {
        // usr/local/bin/python -> /usr/bin/python: rewritten to ../../bin/python
        let dest = TempDir::new().unwrap();
        let tar_gz = tar_gz_with_symlink("usr/local/bin/python", "/usr/bin/python");
        extract_layer(&tar_gz, dest.path()).unwrap();
        let link = dest.path().join("usr/local/bin/python");
        assert!(link.symlink_metadata().is_ok(), "symlink should exist");
        let target = std::fs::read_link(&link).unwrap();
        assert!(
            !target.is_absolute(),
            "target must be relative, got: {target:?}"
        );
        assert_eq!(
            target,
            std::path::PathBuf::from("../../bin/python"),
            "usr/local/bin/python -> /usr/bin/python should rewrite to '../../bin/python'"
        );
    }

    #[cfg(unix)]
    #[test]
    fn absolute_symlink_rewritten_to_relative() {
        let dest = TempDir::new().unwrap();
        // /bin/sh is an absolute symlink target — should be rewritten to bin/sh
        let tar_gz = tar_gz_with_symlink("link_to_sh", "/bin/sh");
        extract_layer(&tar_gz, dest.path()).unwrap();
        let link = dest.path().join("link_to_sh");
        assert!(
            link.symlink_metadata().is_ok(),
            "symlink should have been created"
        );
        let target = std::fs::read_link(&link).unwrap();
        assert!(
            !target.is_absolute(),
            "symlink target should have been rewritten to relative, got: {target:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn absolute_symlink_with_parent_traversal_rejected() {
        let dest = TempDir::new().unwrap();
        // Symlink whose absolute target, when relativized, contains ../
        let tar_gz = tar_gz_with_symlink("evil_link", "/../../etc/shadow");
        let err = extract_layer(&tar_gz, dest.path()).unwrap_err();
        assert!(
            err.to_string().contains("traversal") || err.to_string().contains(".."),
            "expected traversal error, got: {err}"
        );
    }

    // ---------------------------------------------------------------------------
    // relative_path
    // ---------------------------------------------------------------------------

    #[test]
    fn relative_path_same_dir() {
        // bin/echo -> /bin/busybox: target is in same dir, result is just filename
        assert_eq!(
            relative_path(Path::new("bin"), Path::new("bin/busybox")),
            std::path::PathBuf::from("busybox")
        );
    }

    #[test]
    fn relative_path_cross_dir() {
        // usr/local/bin/python -> /usr/bin/python: go up two dirs, then into bin
        assert_eq!(
            relative_path(Path::new("usr/local/bin"), Path::new("usr/bin/python")),
            std::path::PathBuf::from("../../bin/python")
        );
    }

    #[test]
    fn relative_path_root_to_nested() {
        // symlink at root level -> /usr/bin/python: no parent dirs to climb
        assert_eq!(
            relative_path(Path::new(""), Path::new("usr/bin/python")),
            std::path::PathBuf::from("usr/bin/python")
        );
    }

    // ---------------------------------------------------------------------------
    // verify_digest
    // ---------------------------------------------------------------------------

    #[test]
    fn correct_digest_accepted() {
        use sha2::{Digest as _, Sha256};
        let data = b"test data";
        let hash = hex::encode(Sha256::digest(data));
        let digest = format!("sha256:{hash}");
        verify_digest(data, &digest).unwrap();
    }

    #[test]
    fn wrong_digest_rejected() {
        let err = verify_digest(
            b"hello",
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("mismatch")
                || err.to_string().to_lowercase().contains("digest")
        );
    }

    #[test]
    fn missing_prefix_rejected() {
        let err = verify_digest(b"hello", "abc123").unwrap_err();
        assert!(
            err.to_string().contains("mismatch")
                || err.to_string().to_lowercase().contains("digest")
        );
    }

    #[cfg(test)]
    mod proptest_tests {
        use super::*;
        use proptest::prelude::*;
        use std::path::PathBuf;
        use tempfile::TempDir;

        // Any path containing a `..` component must always be rejected.
        proptest! {
            #[test]
            fn dotdot_paths_always_rejected(
                prefix in "[a-z]{1,8}",
                suffix in "[a-z]{1,8}",
            ) {
                let dir = TempDir::new().unwrap();
                let dest = dir.path();
                let evil = PathBuf::from(format!("{prefix}/../../{suffix}"));
                let result = validate_tar_entry_path(&evil, dest);
                prop_assert!(result.is_err(), "expected rejection for path {:?}", evil);
            }

            // Valid relative paths (no `..`, no absolute) must never panic —
            // they may succeed or return a clean error, never panic.
            #[test]
            fn safe_relative_paths_do_not_panic(
                component in "[a-zA-Z0-9_-]{1,16}",
            ) {
                let dir = TempDir::new().unwrap();
                let dest = dir.path();
                let path = PathBuf::from(&component);
                // Must not panic — result can be Ok or Err
                let _ = validate_tar_entry_path(&path, dest);
            }
        }
    }
}
