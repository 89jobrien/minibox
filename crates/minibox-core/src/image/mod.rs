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

        extract_layer(&buf, &dest)
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

    /// Return the path to the stored `manifest.json` for `name:tag`.
    fn manifest_path(&self, name: &str, tag: &str) -> anyhow::Result<PathBuf> {
        Ok(self.image_dir(name, tag)?.join("manifest.json"))
    }

    /// Return the path to the `layers/` subdirectory for `name:tag`.
    fn layers_dir(&self, name: &str, tag: &str) -> anyhow::Result<PathBuf> {
        Ok(self.image_dir(name, tag)?.join("layers"))
    }

    /// Read and deserialize the stored manifest for `name:tag`.
    ///
    /// Returns [`ImageError::NotFound`] if the manifest file does not exist.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::manifest::Descriptor;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;
    use tempfile::TempDir;

    fn sample_manifest(layer_digests: &[&str]) -> OciManifest {
        OciManifest {
            schema_version: 2,
            media_type: "application/vnd.oci.image.manifest.v1+json".to_string(),
            config: Descriptor {
                media_type: "application/vnd.oci.image.config.v1+json".to_string(),
                size: 100,
                digest: "sha256:config123".to_string(),
                platform: None,
            },
            layers: layer_digests
                .iter()
                .map(|d| Descriptor {
                    media_type: "application/vnd.oci.image.layer.v1.tar+gzip".to_string(),
                    size: 1000,
                    digest: d.to_string(),
                    platform: None,
                })
                .collect(),
        }
    }

    fn create_test_layer() -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let data = b"hello from layer";
        let mut header = tar::Header::new_gnu();
        header.set_path("test.txt").expect("set path");
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, &data[..]).expect("append");
        let tar_bytes = builder.into_inner().expect("finish tar");

        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&tar_bytes).expect("write gz");
        encoder.finish().expect("finish gz")
    }

    #[test]
    fn test_new_creates_directory() {
        let tmp = TempDir::new().expect("tempdir");
        let store_path = tmp.path().join("images");
        assert!(!store_path.exists());

        let _store = ImageStore::new(&store_path).expect("ImageStore::new");
        assert!(
            store_path.exists(),
            "base_dir should be created by ImageStore::new"
        );
    }

    #[test]
    fn test_has_image_false_when_empty() {
        let tmp = TempDir::new().expect("tempdir");
        let store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");
        assert!(!store.has_image("alpine", "latest"));
    }

    #[test]
    fn test_store_and_has_manifest() {
        let tmp = TempDir::new().expect("tempdir");
        let store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");

        assert!(!store.has_image("alpine", "latest"));

        let manifest = sample_manifest(&["sha256:layer1"]);
        store
            .store_manifest("alpine", "latest", &manifest)
            .expect("store_manifest");

        assert!(store.has_image("alpine", "latest"));
    }

    #[test]
    fn test_get_image_layers_returns_correct_paths() {
        let tmp = TempDir::new().expect("tempdir");
        let store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");

        let manifest = sample_manifest(&["sha256:aaa111", "sha256:bbb222"]);
        store
            .store_manifest("myimage", "v1", &manifest)
            .expect("store_manifest");

        let layers = store
            .get_image_layers("myimage", "v1")
            .expect("get_image_layers");
        assert_eq!(layers.len(), 2);

        // Digest colons are replaced with underscores in the path
        assert!(
            layers[0].to_string_lossy().contains("sha256_aaa111"),
            "first layer path should contain sha256_aaa111, got: {:?}",
            layers[0]
        );
        assert!(
            layers[1].to_string_lossy().contains("sha256_bbb222"),
            "second layer path should contain sha256_bbb222, got: {:?}",
            layers[1]
        );
    }

    #[test]
    fn test_get_image_layers_not_found() {
        let tmp = TempDir::new().expect("tempdir");
        let store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");

        let result = store.get_image_layers("nonexistent", "latest");
        assert!(result.is_err(), "expected error for missing image");
    }

    #[test]
    fn test_store_layer_extracts_content() {
        let tmp = TempDir::new().expect("tempdir");
        let store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");

        let layer_data = create_test_layer();
        let digest = "sha256:testlayer001";

        let dest = store
            .store_layer("alpine", "latest", digest, std::io::Cursor::new(layer_data))
            .expect("store_layer");

        assert!(dest.exists(), "layer dest directory should exist");
        let extracted_file = dest.join("test.txt");
        assert!(
            extracted_file.exists(),
            "test.txt should be extracted from tar layer"
        );
        let content = std::fs::read_to_string(&extracted_file).expect("read test.txt");
        assert_eq!(content, "hello from layer");
    }

    #[test]
    fn test_store_layer_skips_existing() {
        let tmp = TempDir::new().expect("tempdir");
        let store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");

        let layer_data = create_test_layer();
        let digest = "sha256:deduplayer";

        // First store
        let dest1 = store
            .store_layer(
                "alpine",
                "latest",
                digest,
                std::io::Cursor::new(layer_data.clone()),
            )
            .expect("first store_layer");

        // Write a sentinel file to detect if re-extraction happened
        let sentinel = dest1.join("sentinel.txt");
        std::fs::write(&sentinel, "original").expect("write sentinel");

        // Second store — should be a no-op
        let dest2 = store
            .store_layer("alpine", "latest", digest, std::io::Cursor::new(layer_data))
            .expect("second store_layer");

        assert_eq!(dest1, dest2, "both calls should return the same path");
        let content = std::fs::read_to_string(&sentinel).expect("read sentinel");
        assert_eq!(
            content, "original",
            "sentinel should not be overwritten on second store"
        );
    }

    #[test]
    fn test_image_dir_rejects_path_traversal() {
        let tmp = TempDir::new().expect("tempdir");
        let store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");

        let result = store.has_image("../evil", "latest");
        assert!(!result, "has_image should return false for traversal name");

        let result = store.store_manifest("../evil", "latest", &sample_manifest(&[]));
        assert!(
            result.is_err(),
            "store_manifest should error on path traversal name"
        );
    }

    #[test]
    fn test_image_dir_rejects_empty_name() {
        let tmp = TempDir::new().expect("tempdir");
        let store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");

        let result = store.store_manifest("", "latest", &sample_manifest(&[]));
        assert!(result.is_err(), "store_manifest should error on empty name");

        let result = store.store_manifest("alpine", "", &sample_manifest(&[]));
        assert!(result.is_err(), "store_manifest should error on empty tag");
    }

    #[test]
    fn test_image_dir_rejects_null_bytes() {
        let tmp = TempDir::new().expect("tempdir");
        let store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");

        let result = store.store_manifest("alp\0ine", "latest", &sample_manifest(&[]));
        assert!(
            result.is_err(),
            "store_manifest should error on null byte in name"
        );
    }

    #[test]
    fn test_image_dir_rejects_absolute_path() {
        let tmp = TempDir::new().expect("tempdir");
        let store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");

        let result = store.store_manifest("/etc/passwd", "latest", &sample_manifest(&[]));
        assert!(
            result.is_err(),
            "store_manifest should error on absolute path name"
        );
    }

    #[test]
    fn test_image_dir_replaces_slashes() {
        let tmp = TempDir::new().expect("tempdir");
        let store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");

        let manifest = sample_manifest(&[]);
        store
            .store_manifest("library/ubuntu", "20.04", &manifest)
            .expect("store_manifest with slash in name");

        // Verify the directory uses underscore, not a nested path component
        let expected_dir = store.base_dir.join("library_ubuntu").join("20.04");
        assert!(
            expected_dir.exists(),
            "expected directory library_ubuntu/20.04 to exist, got: {:?}",
            expected_dir
        );

        assert!(store.has_image("library/ubuntu", "20.04"));
    }
}
