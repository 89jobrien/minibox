//! Overlay filesystem commit adapter.
//!
//! Snapshots a container's writable layer (upperdir) into a new OCI image
//! by tarring the upperdir, storing it as a new layer blob, and constructing
//! a new OCI manifest.

use crate::daemonbox_state::StateHandle;
use anyhow::{Context, Result};
use async_trait::async_trait;
use minibox_core::as_any;
use minibox_core::domain::{
    CommitConfig, ContainerCommitter, ContainerId, DynContainerCommitter, ImageMetadata, LayerInfo,
};
use minibox_core::image::ImageStore;
use minibox_core::image::manifest::{Descriptor, OciManifest};
use std::sync::Arc;

pub struct OverlayCommitAdapter {
    image_store: Arc<ImageStore>,
    state: StateHandle,
}

impl OverlayCommitAdapter {
    pub fn new(image_store: Arc<ImageStore>, state: StateHandle) -> Self {
        Self { image_store, state }
    }
}

as_any!(OverlayCommitAdapter);

#[async_trait]
impl ContainerCommitter for OverlayCommitAdapter {
    async fn commit(
        &self,
        container_id: &ContainerId,
        target_ref: &str,
        config: &CommitConfig,
    ) -> Result<ImageMetadata> {
        let id = container_id.as_str().to_string();

        let upper_dir = self
            .state
            .get_overlay_upper(&id)
            .await
            .with_context(|| format!("container {id} has no overlay upper dir"))?;
        let image_store = Arc::clone(&self.image_store);
        let target_ref = target_ref.to_string();
        let config = config.clone();

        tokio::task::spawn_blocking(move || {
            commit_upper_dir_to_image(image_store, &upper_dir, &target_ref, &config)
        })
        .await
        .context("spawn_blocking commit")?
    }
}

pub fn commit_upper_dir_to_image(
    image_store: Arc<ImageStore>,
    upper_dir: &std::path::Path,
    target_ref: &str,
    config: &CommitConfig,
) -> Result<ImageMetadata> {
    let tar_bytes = tar_directory(upper_dir)?;
    let size = tar_bytes.len() as u64;

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(&tar_bytes);
    let layer_digest = format!("sha256:{:x}", hasher.finalize());

    let (target_name, target_tag) = parse_image_ref(target_ref);

    let layer_dir = image_store
        .layers_dir_pub(&target_name, &target_tag)
        .context("layers_dir")?;
    std::fs::create_dir_all(&layer_dir).context("create layers dir")?;
    let digest_key = layer_digest.replace(':', "_");
    let layer_path = layer_dir.join(format!("{digest_key}.tar"));
    std::fs::write(&layer_path, &tar_bytes).context("write layer tar")?;

    let config_json = serde_json::json!({
        "architecture": "amd64",
        "os": "linux",
        "config": {
            "Env": config.env_overrides,
            "Cmd": config.cmd_override.clone().unwrap_or_default(),
        }
    });
    let config_bytes = serde_json::to_vec(&config_json).context("serialize config")?;
    let mut cfg_hasher = Sha256::new();
    cfg_hasher.update(&config_bytes);
    let config_digest = format!("sha256:{:x}", cfg_hasher.finalize());
    let config_path = layer_dir.join("config.json");
    std::fs::write(&config_path, &config_bytes).context("write config")?;

    let new_manifest = OciManifest {
        schema_version: 2,
        media_type: "application/vnd.oci.image.manifest.v1+json".to_string(),
        config: Descriptor {
            media_type: "application/vnd.oci.image.config.v1+json".to_string(),
            size: config_bytes.len() as u64,
            digest: config_digest,
            platform: None,
        },
        layers: vec![Descriptor {
            media_type: "application/vnd.oci.image.layer.v1.tar".to_string(),
            size,
            digest: layer_digest.clone(),
            platform: None,
        }],
    };

    image_store
        .store_manifest(&target_name, &target_tag, &new_manifest)
        .context("store new manifest")?;

    Ok(ImageMetadata {
        name: target_name,
        tag: target_tag,
        layers: vec![LayerInfo {
            digest: layer_digest,
            size,
        }],
    })
}

fn tar_directory(dir: &std::path::Path) -> Result<Vec<u8>> {
    use tar::Builder;
    let mut buf = Vec::new();
    {
        let mut ar = Builder::new(&mut buf);
        ar.append_dir_all(".", dir)
            .with_context(|| format!("tar {}", dir.display()))?;
        ar.finish().context("tar finish")?;
    }
    Ok(buf)
}

fn parse_image_ref(s: &str) -> (String, String) {
    if let Some((name, tag)) = s.rsplit_once(':') {
        (name.to_string(), tag.to_string())
    } else {
        (s.to_string(), "latest".to_string())
    }
}

pub fn overlay_commit_adapter(
    image_store: Arc<ImageStore>,
    state: StateHandle,
) -> DynContainerCommitter {
    Arc::new(OverlayCommitAdapter::new(image_store, state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_image_ref_with_tag() {
        let (name, tag) = parse_image_ref("myapp:v1.2");
        assert_eq!(name, "myapp");
        assert_eq!(tag, "v1.2");
    }

    #[test]
    fn parse_image_ref_no_tag() {
        let (name, tag) = parse_image_ref("myapp");
        assert_eq!(name, "myapp");
        assert_eq!(tag, "latest");
    }

    #[test]
    fn tar_empty_dir_produces_bytes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bytes = tar_directory(tmp.path()).unwrap();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn commit_upper_dir_produces_correct_metadata() {
        let tmp = tempfile::TempDir::new().unwrap();
        let upper_dir = tmp.path().join("upper");
        std::fs::create_dir_all(&upper_dir).unwrap();
        std::fs::write(upper_dir.join("hello.txt"), b"hello").unwrap();

        let images_dir = tmp.path().join("images");
        let image_store =
            Arc::new(minibox_core::image::ImageStore::new(&images_dir).expect("image store"));

        let meta = commit_upper_dir_to_image(
            image_store,
            &upper_dir,
            "myapp:v1",
            &CommitConfig {
                author: None,
                message: None,
                env_overrides: vec![],
                cmd_override: None,
            },
        )
        .expect("commit");

        assert_eq!(meta.name, "myapp");
        assert_eq!(meta.tag, "v1");
        assert_eq!(meta.layers.len(), 1);
        assert!(
            meta.layers[0].digest.starts_with("sha256:"),
            "digest should be sha256: prefixed"
        );
        assert!(meta.layers[0].size > 0, "layer size should be non-zero");
    }

    #[test]
    fn commit_preserves_layer_digest_across_identical_contents() {
        let tmp = tempfile::TempDir::new().unwrap();
        let upper_dir = tmp.path().join("upper");
        std::fs::create_dir_all(&upper_dir).unwrap();
        std::fs::write(upper_dir.join("file.txt"), b"deterministic").unwrap();

        let images_dir = tmp.path().join("images");
        let image_store =
            Arc::new(minibox_core::image::ImageStore::new(&images_dir).expect("image store"));
        let config = CommitConfig {
            author: None,
            message: None,
            env_overrides: vec![],
            cmd_override: None,
        };

        let meta1 =
            commit_upper_dir_to_image(Arc::clone(&image_store), &upper_dir, "app:a", &config)
                .expect("commit 1");
        let meta2 =
            commit_upper_dir_to_image(Arc::clone(&image_store), &upper_dir, "app:b", &config)
                .expect("commit 2");

        assert_eq!(
            meta1.layers[0].digest, meta2.layers[0].digest,
            "identical content should produce identical layer digest"
        );
    }
}
