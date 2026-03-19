//! Image store: local persistence of pulled OCI images.
//!
//! Images are stored under `{base_dir}/{name}/{tag}/`:
//! - `manifest.json` -- the OCI manifest JSON blob.
//! - `layers/{digest}/` -- one directory per layer, containing the extracted
//!   tar contents.
//!
//! [`ImageStore`] is the main entry point.

pub mod layer;
pub mod manifest;
pub mod registry;

use crate::error::ImageError;
use crate::image::layer::extract_layer;
use crate::image::manifest::OciManifest;
use anyhow::Context;
use std::io::Read;
use std::path::PathBuf;
use tracing::{debug, info};

/// Local image store backed by a directory on disk.
///
/// Typical base directory: `/var/lib/minibox/images`.
#[derive(Debug, Clone)]
pub struct ImageStore {
    /// Root directory of the image store.
    pub base_dir: PathBuf,
}

impl ImageStore {
    /// Create an [`ImageStore`] pointing at `base_dir`.
    ///
    /// The directory is created if it does not already exist.
    pub fn new(base_dir: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let base_dir = base_dir.into();
        std::fs::create_dir_all(&base_dir).map_err(|source| ImageError::StoreWrite {
            path: base_dir.display().to_string(),
            source,
        })?;
        Ok(Self { base_dir })
    }

    // -----------------------------------------------------------------------
    // Query
    // -----------------------------------------------------------------------

    /// Returns `true` if the image `name:tag` has been pulled and its manifest
    /// is present on disk.
    pub fn has_image(&self, name: &str, tag: &str) -> bool {
        self.manifest_path(name, tag)
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// Return the ordered list of layer directories for `name:tag`
    /// (bottom-to-top, matching the order in the manifest).
    pub fn get_image_layers(&self, name: &str, tag: &str) -> anyhow::Result<Vec<PathBuf>> {
        let manifest = self.load_manifest(name, tag)?;
        let layers_base = self.layers_dir(name, tag)?;

        let paths: Vec<PathBuf> = manifest
            .layers
            .iter()
            .map(|desc| {
                // Digest format: "sha256:<hex>"
                let digest_key = desc.digest.replace(':', "_");
                layers_base.join(&digest_key)
            })
            .collect();

        Ok(paths)
    }

    // -----------------------------------------------------------------------
    // Write
    // -----------------------------------------------------------------------

    /// Persist the OCI manifest for `name:tag`.
    pub fn store_manifest(
        &self,
        name: &str,
        tag: &str,
        manifest: &OciManifest,
    ) -> anyhow::Result<()> {
        let path = self.manifest_path(name, tag)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ImageError::StoreWrite {
                path: parent.display().to_string(),
                source,
            })?;
        }

        let json =
            serde_json::to_string_pretty(manifest).map_err(|source| ImageError::ManifestParse {
                name: name.to_owned(),
                tag: tag.to_owned(),
                source,
            })?;

        std::fs::write(&path, json).map_err(|source| ImageError::StoreWrite {
            path: path.display().to_string(),
            source,
        })?;

        info!("stored manifest for {}:{} at {:?}", name, tag, path);
        Ok(())
    }

    /// Extract a gzip-compressed tar layer blob into the store and return the
    /// directory path.
    ///
    /// `data_reader` is consumed and its contents are extracted into
    /// `{layers_dir}/{digest_key}/`. The digest is NOT verified here -- call
    /// [`layer::verify_digest`] before passing data if you need verification.
    pub fn store_layer<R: Read>(
        &self,
        name: &str,
        tag: &str,
        digest: &str,
        mut data_reader: R,
    ) -> anyhow::Result<PathBuf> {
        let digest_key = digest.replace(':', "_");
        let dest = self.layers_dir(name, tag)?.join(&digest_key);

        if dest.exists() {
            debug!("layer {} already extracted at {:?}, skipping", digest, dest);
            return Ok(dest);
        }

        std::fs::create_dir_all(&dest).map_err(|source| ImageError::StoreWrite {
            path: dest.display().to_string(),
            source,
        })?;

        // Read all bytes (needed for the extractor which needs full data).
        let mut buf = Vec::new();
        data_reader
            .read_to_end(&mut buf)
            .map_err(|source| ImageError::StoreRead {
                path: digest.to_owned(),
                source,
            })?;

        extract_layer(buf.as_slice(), &dest)
            .with_context(|| format!("extracting layer {digest} to {dest:?}"))?;

        info!("stored layer {} at {:?}", digest, dest);
        Ok(dest)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Get the directory path for an image, with security validation.
    ///
    /// # Security
    ///
    /// Validates that the image name and tag don't contain path traversal
    /// sequences or other dangerous characters.
    fn image_dir(&self, name: &str, tag: &str) -> anyhow::Result<PathBuf> {
        // SECURITY: Validate name and tag
        for (component_name, component) in [("image name", name), ("tag", tag)] {
            // Reject empty strings
            if component.is_empty() {
                return Err(ImageError::Other(format!(
                    "invalid {component_name}: cannot be empty"
                ))
                .into());
            }

            // Reject absolute paths
            if component.starts_with('/') {
                return Err(ImageError::Other(format!(
                    "invalid {component_name}: cannot start with /"
                ))
                .into());
            }

            // Reject path traversal
            if component.contains("..") {
                return Err(ImageError::Other(format!(
                    "invalid {component_name}: cannot contain '..'"
                ))
                .into());
            }

            // Reject null bytes
            if component.contains('\0') {
                return Err(ImageError::Other(format!(
                    "invalid {component_name}: cannot contain null bytes"
                ))
                .into());
            }
        }

        // Replace '/' in image name (e.g. "library/ubuntu") with '_'
        let safe_name = name.replace('/', "_");
        let result = self.base_dir.join(&safe_name).join(tag);

        // SECURITY: Canonicalize and verify the result is still under base_dir
        // Note: The path may not exist yet, so we validate the parent instead
        if let Some(parent) = result.parent()
            && parent.exists()
        {
            let canonical = parent
                .canonicalize()
                .with_context(|| format!("canonicalizing parent of image dir: {parent:?}"))?;
            let canonical_base = self
                .base_dir
                .canonicalize()
                .with_context(|| format!("canonicalizing base dir: {:?}", self.base_dir))?;

            if !canonical.starts_with(&canonical_base) {
                return Err(ImageError::Other(format!(
                    "path traversal attempt: image dir {canonical:?} is outside base {canonical_base:?}"
                ))
                .into());
            }
        }

        debug!("validated image_dir for {}:{}: {:?}", name, tag, result);
        Ok(result)
    }

    fn manifest_path(&self, name: &str, tag: &str) -> anyhow::Result<PathBuf> {
        Ok(self.image_dir(name, tag)?.join("manifest.json"))
    }

    fn layers_dir(&self, name: &str, tag: &str) -> anyhow::Result<PathBuf> {
        Ok(self.image_dir(name, tag)?.join("layers"))
    }

    fn load_manifest(&self, name: &str, tag: &str) -> anyhow::Result<OciManifest> {
        let path = self.manifest_path(name, tag)?;
        if !path.exists() {
            return Err(ImageError::NotFound {
                name: name.to_owned(),
                tag: tag.to_owned(),
            }
            .into());
        }
        let data = std::fs::read(&path).map_err(|source| ImageError::StoreRead {
            path: path.display().to_string(),
            source,
        })?;
        serde_json::from_slice(&data).map_err(|source| {
            anyhow::Error::from(ImageError::ManifestParse {
                name: name.to_owned(),
                tag: tag.to_owned(),
                source,
            })
        })
    }
}
