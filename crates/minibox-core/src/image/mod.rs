//! Image store: local persistence of pulled OCI images.
//!
//! Images are stored under `{base_dir}/{name}/{tag}/`:
//! - `manifest.json` -- the OCI manifest JSON blob.
//! - `layers/{digest}/` -- one directory per layer, containing the extracted
//!   tar contents.
//!
//! [`ImageStore`] is the main entry point.

pub mod dockerfile;
pub mod gc;
pub mod layer;
pub mod lease;
pub mod manifest;
pub mod reference;
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
    ///
    /// Uses a fast-path that avoids the full `image_dir` security machinery
    /// (canonicalize, anyhow chain) since this is a read-only existence check
    /// on a caller-supplied name that has already been validated upstream.
    /// Dangerous-char rejection is still performed inline.
    pub fn has_image(&self, name: &str, tag: &str) -> bool {
        // Inline basic safety checks — reject traversal / absolute / empty.
        for component in [name, tag] {
            if component.is_empty()
                || component.starts_with('/')
                || component.contains("..")
                || component.contains('\0')
            {
                return false;
            }
        }
        let safe_name = name.replace('/', "_");
        self.base_dir
            .join(safe_name)
            .join(tag)
            .join("manifest.json")
            .exists()
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

    /// List all `"name:tag"` strings known to this store.
    ///
    /// Walks `{base_dir}/*/` directories looking for `manifest.json`.
    pub async fn list_all_images(&self) -> anyhow::Result<Vec<String>> {
        let mut result = Vec::new();
        let mut rd = tokio::fs::read_dir(&self.base_dir).await?;
        while let Some(name_entry) = rd.next_entry().await? {
            if !name_entry.file_type().await?.is_dir() {
                continue;
            }
            let name = name_entry.file_name().to_string_lossy().replace('_', "/");
            let mut td = tokio::fs::read_dir(name_entry.path()).await?;
            while let Some(tag_entry) = td.next_entry().await? {
                if !tag_entry.file_type().await?.is_dir() {
                    continue;
                }
                let manifest = tag_entry.path().join("manifest.json");
                if manifest.exists() {
                    let tag = tag_entry.file_name().to_string_lossy().to_string();
                    result.push(format!("{name}:{tag}"));
                }
            }
        }
        Ok(result)
    }

    /// Return the total disk usage of an image's layer dirs in bytes.
    pub async fn image_size_bytes(&self, name: &str, tag: &str) -> anyhow::Result<u64> {
        let dir = self.image_dir(name, tag)?;
        let mut total = 0u64;
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            let mut rd = tokio::fs::read_dir(&d).await?;
            while let Some(e) = rd.next_entry().await? {
                let meta = e.metadata().await?;
                if meta.is_dir() {
                    stack.push(e.path());
                } else {
                    total += meta.len();
                }
            }
        }
        Ok(total)
    }

    /// Delete an image's manifest and all layer directories.
    ///
    /// Best-effort: logs a warning if the directory cannot be removed.
    pub async fn delete_image(&self, name: &str, tag: &str) -> anyhow::Result<()> {
        let dir = self.image_dir(name, tag)?;
        if dir.exists() {
            tokio::fs::remove_dir_all(&dir)
                .await
                .with_context(|| format!("image: remove_dir_all {}", dir.display()))?;
            info!(image = %format!("{name}:{tag}"), "image: deleted");
        }
        Ok(())
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

        extract_layer(&mut data_reader, &dest)
            .with_context(|| format!("extracting layer {digest} to {dest:?}"))?;

        info!("stored layer {} at {:?}", digest, dest);
        Ok(dest)
    }

    /// Extract a gzip-compressed tar layer blob with digest verification and
    /// atomic commit.
    ///
    /// Unlike [`store_layer`](Self::store_layer), this method:
    /// 1. Wraps `data_reader` in a [`HashingReader`](layer::HashingReader) to
    ///    compute the SHA-256 of the compressed stream.
    /// 2. Extracts into a temporary sibling directory (`{digest_key}.tmp`).
    /// 3. Verifies the computed digest matches `expected_digest`.
    /// 4. Atomically renames the tmp dir to the final location on success.
    /// 5. Cleans up the tmp dir on any failure (extraction or digest mismatch).
    ///
    /// Returns the final layer directory path.
    pub fn store_layer_verified<R: Read>(
        &self,
        name: &str,
        tag: &str,
        expected_digest: &str,
        data_reader: R,
    ) -> anyhow::Result<PathBuf> {
        use crate::image::layer::HashingReader;

        let digest_key = expected_digest.replace(':', "_");
        let layers_base = self.layers_dir(name, tag)?;
        let layer_dir = layers_base.join(&digest_key);

        // Early exit if already cached.
        if layer_dir.exists() {
            debug!(
                digest = %expected_digest,
                path = %layer_dir.display(),
                "layer: already cached, skipping"
            );
            return Ok(layer_dir);
        }

        let tmp_dir = layers_base.join(format!("{digest_key}.tmp"));

        // Clean up any stale tmp dir from a previous failed attempt.
        if tmp_dir.exists() {
            std::fs::remove_dir_all(&tmp_dir)
                .with_context(|| format!("remove stale tmp {}", tmp_dir.display()))?;
        }

        std::fs::create_dir_all(&tmp_dir).map_err(|source| ImageError::StoreWrite {
            path: tmp_dir.display().to_string(),
            source,
        })?;

        let mut hashing_reader = HashingReader::new(data_reader);

        // Extract into tmp dir.
        let extract_result = extract_layer(&mut hashing_reader, &tmp_dir);

        // Drain remaining bytes so the hash covers the full compressed stream.
        if extract_result.is_err() {
            let _ = std::io::copy(&mut hashing_reader, &mut std::io::sink());
        }

        // Verify digest before committing.
        let actual_hex = hashing_reader.finalize();
        let expected_hex = expected_digest
            .strip_prefix("sha256:")
            .ok_or_else(|| anyhow::anyhow!("digest missing sha256: prefix: {expected_digest}"))?;

        if actual_hex != expected_hex {
            if let Err(ce) = std::fs::remove_dir_all(&tmp_dir) {
                tracing::warn!(
                    digest = %expected_digest,
                    error = %ce,
                    "layer: failed to clean up tmp dir after digest mismatch"
                );
            }
            return Err(ImageError::DigestMismatch {
                digest: expected_digest.to_owned(),
                expected: expected_hex.to_owned(),
                actual: actual_hex,
            }
            .into());
        }

        // Digest matched -- surface any extraction error now.
        if let Err(e) = extract_result {
            if let Err(ce) = std::fs::remove_dir_all(&tmp_dir) {
                tracing::warn!(
                    digest = %expected_digest,
                    error = %ce,
                    "layer: failed to clean up tmp dir after extract error"
                );
            }
            return Err(e).with_context(|| format!("extracting layer {expected_digest}"));
        }

        // Atomic rename: tmp -> final dest.
        if let Err(e) = std::fs::rename(&tmp_dir, &layer_dir) {
            // Another concurrent caller may have won the race.
            if layer_dir.exists() {
                let _ = std::fs::remove_dir_all(&tmp_dir);
                return Ok(layer_dir);
            }
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Err(e).with_context(|| {
                format!("rename {} -> {}", tmp_dir.display(), layer_dir.display())
            });
        }

        info!(
            digest = %expected_digest,
            path = %layer_dir.display(),
            "layer: stored with verified digest"
        );
        Ok(layer_dir)
    }

    // -----------------------------------------------------------------------
    // Public helpers for push adapter
    // -----------------------------------------------------------------------

    /// Load and deserialize the stored manifest for `name:tag`.
    ///
    /// Public wrapper of the private [`Self::load_manifest`] used by the push
    /// adapter to enumerate layer digests before uploading.
    pub fn load_manifest_pub(&self, name: &str, tag: &str) -> anyhow::Result<OciManifest> {
        self.load_manifest(name, tag)
    }

    /// Return the path to the `layers/` subdirectory for `name:tag`.
    ///
    /// Public wrapper of the private [`Self::layers_dir`] used by the push
    /// adapter to locate extracted layer directories.
    pub fn layers_dir_pub(&self, name: &str, tag: &str) -> anyhow::Result<std::path::PathBuf> {
        self.layers_dir(name, tag)
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

/// Pull an OCI image into a local store.
///
/// Parses `image_ref` (e.g. `"alpine:latest"`, `"ghcr.io/org/name:tag"`),
/// authenticates against the appropriate registry, downloads the manifest and
/// all layer blobs, and extracts them under `store_path`.
///
/// `progress` is called with a human-readable status string for each major
/// step (auth, manifest fetch, each layer).  Pass a no-op closure to silence
/// progress output.
pub async fn pull(
    image_ref: &str,
    store_path: impl Into<std::path::PathBuf>,
    mut progress: impl FnMut(&str),
) -> anyhow::Result<()> {
    use reference::ImageRef;
    use registry::RegistryClient;

    let image_ref = ImageRef::parse(image_ref)
        .map_err(|e| anyhow::anyhow!("invalid image reference {image_ref:?}: {e}"))?;

    let store = ImageStore::new(store_path).context("opening image store")?;
    let client = RegistryClient::new().context("creating registry client")?;

    let name = image_ref.cache_name();
    let tag = &image_ref.tag;

    if store.has_image(&name, tag) {
        progress(&format!("image {name}:{tag} already cached"));
        return Ok(());
    }

    progress(&format!("authenticating for {name}"));
    let token = client
        .authenticate(&image_ref.repository())
        .await
        .with_context(|| format!("authenticating for {name}"))?;

    progress(&format!("fetching manifest for {name}:{tag}"));
    let manifest = client
        .get_manifest(&image_ref.repository(), tag, &token)
        .await
        .with_context(|| format!("fetching manifest for {name}:{tag}"))?;

    let layer_count = manifest.layers.len();
    for (i, layer) in manifest.layers.iter().enumerate() {
        progress(&format!(
            "pulling layer {}/{}: {}",
            i + 1,
            layer_count,
            &layer.digest[..layer.digest.len().min(20)]
        ));
        let blob = client
            .pull_layer(&image_ref.repository(), &layer.digest, &token)
            .await
            .with_context(|| format!("fetching layer {}", layer.digest))?;
        store
            .store_layer(&name, tag, &layer.digest, std::io::Cursor::new(&blob[..]))
            .with_context(|| format!("storing layer {}", layer.digest))?;
    }

    store
        .store_manifest(&name, tag, &manifest)
        .context("storing manifest")?;

    progress(&format!("pulled {name}:{tag} ({layer_count} layers)"));
    Ok(())
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

    #[tokio::test]
    async fn test_list_all_images_empty() {
        let tmp = TempDir::new().unwrap();
        let store = ImageStore::new(tmp.path()).unwrap();
        let images = store.list_all_images().await.unwrap();
        assert!(images.is_empty());
    }

    #[tokio::test]
    async fn test_delete_image_removes_dir() {
        let tmp = TempDir::new().unwrap();
        let store = ImageStore::new(tmp.path()).unwrap();
        // Seed a fake image dir
        let img_dir = tmp.path().join("alpine").join("latest");
        tokio::fs::create_dir_all(&img_dir).await.unwrap();
        tokio::fs::write(img_dir.join("manifest.json"), b"{}")
            .await
            .unwrap();

        store.delete_image("alpine", "latest").await.unwrap();

        assert!(!img_dir.exists());
    }

    #[tokio::test]
    async fn test_image_size_bytes() {
        let tmp = TempDir::new().unwrap();
        let store = ImageStore::new(tmp.path().join("images")).unwrap();

        let manifest = sample_manifest(&["sha256:layer1"]);
        store.store_manifest("alpine", "latest", &manifest).unwrap();

        let size = store.image_size_bytes("alpine", "latest").await.unwrap();
        // Should include the manifest.json at least
        assert!(size > 0, "image_size_bytes should be > 0 for stored image");
    }
}
