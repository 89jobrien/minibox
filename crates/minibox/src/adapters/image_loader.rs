//! Native ImageLoader adapter — extracts a local OCI tarball into ImageStore.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use minibox_core::domain::ImageLoader;
use minibox_core::image::ImageStore;
use minibox_core::image::manifest::OciManifest;
use std::path::Path;
use std::sync::Arc;

pub struct NativeImageLoader {
    store: Arc<ImageStore>,
}

impl NativeImageLoader {
    pub fn new(store: Arc<ImageStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl ImageLoader for NativeImageLoader {
    async fn load_image(&self, path: &Path, name: &str, tag: &str) -> Result<()> {
        if !path.exists() {
            bail!("image tarball not found: {}", path.display());
        }

        // Unpack the outer OCI layout tarball into a temp dir
        let file = std::fs::File::open(path)
            .with_context(|| format!("open tarball {}", path.display()))?;
        let mut outer = tar::Archive::new(file);
        let tmp = tempfile::TempDir::new().context("create temp dir")?;
        outer.unpack(tmp.path()).context("unpack OCI tarball")?;

        // Parse manifest.json for layer digests
        let manifest_path = tmp.path().join("manifest.json");
        let manifest_bytes = std::fs::read(&manifest_path)
            .with_context(|| format!("read {}", manifest_path.display()))?;
        let manifest: OciManifest =
            serde_json::from_slice(&manifest_bytes).context("parse manifest.json")?;

        // Store manifest
        self.store
            .store_manifest(name, tag, &manifest)
            .with_context(|| format!("store manifest for {name}:{tag}"))?;

        // Extract each layer blob into the image store
        for layer_desc in &manifest.layers {
            let digest_hex = layer_desc.digest.trim_start_matches("sha256:");
            let blob_path = tmp.path().join("blobs").join("sha256").join(digest_hex);
            let blob_file = std::fs::File::open(&blob_path)
                .with_context(|| format!("open blob {}", blob_path.display()))?;
            self.store
                .store_layer(name, tag, &layer_desc.digest, blob_file)
                .with_context(|| format!("store layer {}", layer_desc.digest))?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn load_image_rejects_nonexistent_path() {
        let tmp = TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(tmp.path().join("images")).unwrap());
        let loader = NativeImageLoader::new(store);
        let result = loader
            .load_image(Path::new("/nonexistent/fake.tar"), "test", "latest")
            .await;
        assert!(result.is_err());
    }
}
