//! OCI layer extraction and digest verification.
//!
//! OCI layers are gzip-compressed tar archives. This module decompresses and
//! extracts them into a target directory, and verifies their `sha256:` digest.

use crate::error::ImageError;
use anyhow::Context;
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::path::Path;
use tar::Archive;
use tracing::{debug, info};

/// Extract a gzip-compressed tar layer into `dest`.
///
/// Any files inside the tar are extracted relative to `dest`. The destination
/// directory must already exist.
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

    // Preserve permissions and ownership (requires root in production).
    archive.set_preserve_permissions(true);
    archive.set_preserve_mtime(true);
    archive.set_overwrite(true);

    archive
        .unpack(dest)
        .map_err(|e| ImageError::LayerExtract(format!("tar unpack to {:?} failed: {e}", dest)))
        .with_context(|| format!("extracting layer to {:?}", dest))?;

    info!("layer extracted to {:?}", dest);
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
