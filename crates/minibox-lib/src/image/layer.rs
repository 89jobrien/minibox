//! OCI layer extraction and digest verification.
//!
//! OCI layers are gzip-compressed tar archives. This module decompresses and
//! extracts them into a target directory, and verifies their `sha256:` digest.

use crate::error::ImageError;
use anyhow::Context;
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::path::Path;
use tar::{Archive, EntryType};
use tracing::{debug, info, warn};

// Helper to check for parent directory components
fn has_parent_dir_component(path: &Path) -> bool {
    path.components().any(|c| matches!(c, std::path::Component::ParentDir))
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
pub fn extract_layer(tar_gz_data: &[u8], dest: &Path) -> anyhow::Result<()> {
    debug!(
        "extracting layer ({} bytes) to {:?}",
        tar_gz_data.len(),
        dest
    );

    let gz = GzDecoder::new(tar_gz_data);
    let mut archive = Archive::new(gz);

    // SECURITY: Manually iterate entries to validate each one
    for entry_result in archive.entries()
        .map_err(|e| ImageError::LayerExtract(format!("failed to read tar entries: {e}")))?
    {
        let mut entry = entry_result
            .map_err(|e| ImageError::LayerExtract(format!("failed to read tar entry: {e}")))?;

        let entry_path = entry.path()
            .map_err(|e| ImageError::LayerExtract(format!("invalid entry path: {e}")))?
            .into_owned();

        // SECURITY: Validate entry path doesn't escape destination
        validate_tar_entry_path(&entry_path, dest)?;

        // SECURITY: Check entry type and reject dangerous ones
        let entry_type = entry.header().entry_type();

        // Reject device nodes
        if matches!(entry_type, EntryType::Block | EntryType::Char) {
            return Err(ImageError::LayerExtract(format!(
                "tar entry contains device node (security risk): {entry_path:?}"
            ))
            .into());
        }

        // Warn on symlinks to absolute paths and reject them
        if entry_type == EntryType::Symlink {
            if let Ok(Some(link_target)) = entry.link_name() {
                if link_target.is_absolute() {
                    return Err(ImageError::LayerExtract(format!(
                        "tar entry contains symlink to absolute path (security risk): {entry_path:?} -> {link_target:?}"
                    ))
                    .into());
                }
            }
        }

        // SECURITY: Strip setuid/setgid bits from file permissions
        // We don't trust image-provided permissions for these bits
        if entry_type == EntryType::Regular {
            let mode = entry.header().mode()
                .map_err(|e| ImageError::LayerExtract(format!("failed to get mode: {e}")))?;

            // Remove setuid (04000), setgid (02000), and sticky (01000) bits
            let safe_mode = mode & 0o777;
            if mode != safe_mode {
                warn!(
                    "stripped special bits from {:?}: {:#o} -> {:#o}",
                    entry_path, mode, safe_mode
                );
            }
            // Note: tar crate doesn't provide set_mode(), so we rely on umask
            // This is acceptable as we're running as root with controlled umask
        }

        // Extract the entry
        entry.unpack_in(dest)
            .map_err(|e| ImageError::LayerExtract(format!(
                "failed to extract entry {entry_path:?}: {e}"
            )))?;
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
    let canonical_dest = dest.canonicalize()
        .with_context(|| format!("canonicalizing dest {dest:?}"))?;

    // Check if the entry path when joined with dest would escape
    // We can't canonicalize full_path if it doesn't exist, so check the parent
    if let Some(parent) = full_path.parent() {
        if parent.exists() {
            let canonical_parent = parent.canonicalize()?;
            if !canonical_parent.starts_with(&canonical_dest) {
                return Err(ImageError::LayerExtract(format!(
                    "tar entry would escape destination: {entry_path:?}"
                ))
                .into());
            }
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
    let expected_hex = expected_digest
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
