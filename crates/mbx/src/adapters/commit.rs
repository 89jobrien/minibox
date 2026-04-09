//! Overlay filesystem commit adapter.
//!
//! Snapshots a container's writable layer (upperdir) into a new OCI image
//! by tarring the upperdir, storing it as a new layer blob, and constructing
//! a new OCI manifest.

use anyhow::{Context, Result};
use async_trait::async_trait;
use minibox_core::as_any;
use minibox_core::domain::{
    CommitConfig, ContainerCommitter, ContainerId, DynContainerCommitter, ImageMetadata, LayerInfo,
};
use minibox_core::image::ImageStore;
use minibox_core::image::manifest::{Descriptor, OciManifest};
use std::sync::Arc;
use tracing::info;

use crate::daemonbox_state::StateHandle;

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

        // Tar the upperdir in a blocking task.
        let tar_bytes = tokio::task::spawn_blocking({
            let upper = upper_dir.clone();
            move || tar_directory(&upper)
        })
        .await
        .context("spawn_blocking tar")??;

        let size = tar_bytes.len() as u64;

        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&tar_bytes);
        let layer_digest = format!("sha256:{:x}", hasher.finalize());

        let (target_name, target_tag) = parse_image_ref(target_ref);

        // Store the new layer blob in a subdir of the layers dir.
        let layer_dir = self
            .image_store
            .layers_dir_pub(&target_name, &target_tag)
            .context("layers_dir")?;
        std::fs::create_dir_all(&layer_dir).context("create layers dir")?;
        let digest_key = layer_digest.replace(':', "_");
        let layer_path = layer_dir.join(format!("{digest_key}.tar"));
        std::fs::write(&layer_path, &tar_bytes).context("write layer tar")?;

        // Build config blob.
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
                digest: config_digest.clone(),
                platform: None,
            },
            layers: vec![Descriptor {
                media_type: "application/vnd.oci.image.layer.v1.tar".to_string(),
                size,
                digest: layer_digest.clone(),
                platform: None,
            }],
        };

        self.image_store
            .store_manifest(&target_name, &target_tag, &new_manifest)
            .context("store new manifest")?;

        info!(
            container_id = %id,
            target = %target_ref,
            digest = %layer_digest,
            "commit: complete"
        );

        Ok(ImageMetadata {
            name: target_name,
            tag: target_tag,
            layers: vec![LayerInfo {
                digest: layer_digest,
                size,
            }],
        })
    }
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
}
