//! Overlay filesystem commit adapter.
//!
//! Snapshots a container's writable layer (upperdir) into a new OCI image
//! by tarring the upperdir, storing it as a new layer blob, and constructing
//! a new OCI manifest.

use crate::container_state::StateHandle;
use anyhow::{Context, Result};
use async_trait::async_trait;
use minibox_core::as_any;
use minibox_core::domain::{
    CommitConfig, CommitResult, ContainerCommitter, ContainerId, DynContainerCommitter,
    ImageMetadata, LayerInfo,
};
use minibox_core::image::ImageStore;
use minibox_core::image::manifest::{Descriptor, OciManifest};
use minibox_core::image::reference::ImageRef;
use minibox_core::image::volume::{parse_volume_paths, path_is_volume_masked};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use walkdir::WalkDir;

/// Commits native overlay writable layers into the local image store.
pub struct OverlayCommitAdapter {
    image_store: Arc<ImageStore>,
    state: StateHandle,
}

impl OverlayCommitAdapter {
    /// Creates a commit adapter backed by an image store and daemon state.
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
    ) -> Result<CommitResult> {
        let id = container_id.as_str().to_string();
        let upper_dir = self
            .state
            .get_overlay_upper(&id)
            .await
            .with_context(|| format!("container {id} has no overlay upper dir"))?;
        let source_ref = self.state.get_source_image_ref(&id).await?;
        let declared_volumes = load_declared_volumes(&self.image_store, &source_ref)?;
        let excluded_volume_paths = if config.include_volumes {
            Vec::new()
        } else {
            volume_paths_with_data(&upper_dir, &declared_volumes)?
        };
        let image_store = Arc::clone(&self.image_store);
        let target_ref = target_ref.to_string();
        let config = config.clone();
        let volumes_for_capture = declared_volumes.clone();

        let image = tokio::task::spawn_blocking(move || {
            commit_upper_dir_to_image_with_volumes(
                image_store,
                &upper_dir,
                &target_ref,
                &config,
                &volumes_for_capture,
            )
        })
        .await
        .context("spawn_blocking commit")??;

        Ok(CommitResult {
            image,
            excluded_volume_paths,
        })
    }
}

fn load_declared_volumes(image_store: &ImageStore, source_ref: &str) -> Result<Vec<PathBuf>> {
    if source_ref.is_empty() {
        return Ok(Vec::new());
    }
    let image_ref = ImageRef::parse(source_ref)
        .with_context(|| format!("parse source image reference {source_ref:?}"))?;
    let Ok(config) = image_store.load_config_blob_pub(&image_ref.cache_name(), &image_ref.tag)
    else {
        return Ok(Vec::new());
    };
    parse_volume_paths(&config)
}

fn volume_paths_with_data(root: &Path, volumes: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut populated = Vec::new();
    for volume in volumes {
        let relative = volume.strip_prefix("/").context("absolute volume path")?;
        let candidate = root.join(relative);
        let Ok(metadata) = std::fs::symlink_metadata(&candidate) else {
            continue;
        };
        let contains_data = if metadata.is_dir() {
            std::fs::read_dir(&candidate)
                .with_context(|| format!("read volume path {}", candidate.display()))?
                .next()
                .transpose()?
                .is_some()
        } else {
            true
        };
        if contains_data {
            populated.push(volume.clone());
        }
    }
    Ok(populated)
}

/// Packages an overlay upper directory as a new local image.
pub fn commit_upper_dir_to_image(
    image_store: Arc<ImageStore>,
    upper_dir: &std::path::Path,
    target_ref: &str,
    config: &CommitConfig,
) -> Result<ImageMetadata> {
    commit_upper_dir_to_image_with_volumes(image_store, upper_dir, target_ref, config, &[])
}

pub(super) fn commit_upper_dir_to_image_with_volumes(
    image_store: Arc<ImageStore>,
    upper_dir: &Path,
    target_ref: &str,
    config: &CommitConfig,
    declared_volumes: &[PathBuf],
) -> Result<ImageMetadata> {
    let excluded = if config.include_volumes {
        &[][..]
    } else {
        declared_volumes
    };
    let tar_bytes = tar_directory(upper_dir, excluded)?;
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
            "Volumes": declared_volumes
                .iter()
                .map(|path| (path.to_string_lossy().into_owned(), serde_json::json!({})))
                .collect::<std::collections::BTreeMap<_, _>>(),
        }
    });
    let config_bytes = serde_json::to_vec(&config_json).context("serialize config")?;
    let mut cfg_hasher = Sha256::new();
    cfg_hasher.update(&config_bytes);
    let config_digest = format!("sha256:{:x}", cfg_hasher.finalize());
    let config_path = layer_dir.join("config.json");
    std::fs::write(&config_path, &config_bytes).context("write config")?;
    image_store
        .store_config_blob(&target_name, &target_tag, &config_bytes)
        .context("store committed image config")?;

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

fn tar_directory(dir: &Path, excluded_volumes: &[PathBuf]) -> Result<Vec<u8>> {
    use tar::Builder;
    let mut buf = Vec::new();
    {
        let mut ar = Builder::new(&mut buf);
        // See docker_archive::tar_directory for why this must be false:
        // layers commonly contain symlinks (e.g. etc/mtab -> ../proc/mounts)
        // that only resolve inside a live container mount namespace and
        // would otherwise fail ENOENT when append_dir_all follows them.
        ar.follow_symlinks(false);
        let walker = WalkDir::new(dir)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|entry| {
                entry.path() == dir
                    || entry
                        .path()
                        .strip_prefix(dir)
                        .is_ok_and(|path| !path_is_volume_masked(path, excluded_volumes))
            });
        for entry in walker {
            let entry = entry.with_context(|| format!("walk {}", dir.display()))?;
            let relative = entry
                .path()
                .strip_prefix(dir)
                .context("strip commit root")?;
            if relative.as_os_str().is_empty() || path_is_volume_masked(relative, excluded_volumes)
            {
                continue;
            }
            ar.append_path_with_name(entry.path(), relative)
                .with_context(|| {
                    format!("tar {} as {}", entry.path().display(), relative.display())
                })?;
        }
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

/// Constructs a dynamic native overlay commit adapter.
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
        let bytes = tar_directory(tmp.path(), &[]).unwrap();
        assert!(!bytes.is_empty());
    }

    /// Regression test: root filesystem layers commonly contain symlinks
    /// that only resolve inside a live container mount namespace, e.g.
    /// Alpine's `etc/mtab -> ../proc/mounts`. `tar_directory` must store
    /// such symlinks as-is rather than following them, or `append_dir_all`
    /// fails with ENOENT trying to stat the (host-side) dangling target.
    #[cfg(unix)]
    #[test]
    fn tar_directory_preserves_dangling_symlinks() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::os::unix::fs::symlink("../proc/mounts", tmp.path().join("mtab")).unwrap();
        let bytes = tar_directory(tmp.path(), &[]).expect("tar despite dangling symlink");
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
                include_volumes: false,
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
            include_volumes: false,
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
    fn tar_paths(bytes: &[u8]) -> Vec<PathBuf> {
        let mut archive = tar::Archive::new(bytes);
        archive
            .entries()
            .expect("tar entries")
            .map(|entry| {
                entry
                    .expect("tar entry")
                    .path()
                    .expect("entry path")
                    .into_owned()
            })
            .collect()
    }

    #[test]
    fn masked_volume_data_is_detected_and_excluded() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let volume_dir = tmp.path().join("var/lib/docker");
        std::fs::create_dir_all(&volume_dir).expect("volume dir");
        std::fs::write(volume_dir.join("state.db"), b"data").expect("volume data");
        std::fs::write(tmp.path().join("kept.txt"), b"kept").expect("regular data");
        let volumes = vec![PathBuf::from("/var/lib/docker")];

        assert_eq!(
            volume_paths_with_data(tmp.path(), &volumes).expect("detect data"),
            volumes
        );
        let paths = tar_paths(&tar_directory(tmp.path(), &volumes).expect("capture"));
        assert!(paths.contains(&PathBuf::from("kept.txt")));
        assert!(!paths.iter().any(|path| path.starts_with("var/lib/docker")));
    }

    #[test]
    fn include_volumes_captures_masked_data() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let volume_dir = tmp.path().join("var/lib/docker");
        std::fs::create_dir_all(&volume_dir).expect("volume dir");
        std::fs::write(volume_dir.join("state.db"), b"data").expect("volume data");

        let paths = tar_paths(&tar_directory(tmp.path(), &[]).expect("capture"));
        assert!(paths.contains(&PathBuf::from("var/lib/docker/state.db")));
    }

    #[test]
    fn committed_image_config_preserves_volume_declarations() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let upper_dir = tmp.path().join("upper");
        std::fs::create_dir(&upper_dir).expect("upper dir");
        let store = Arc::new(ImageStore::new(tmp.path().join("images")).expect("image store"));
        let config = CommitConfig {
            author: None,
            message: None,
            env_overrides: vec![],
            cmd_override: None,
            include_volumes: false,
        };
        let volumes = vec![PathBuf::from("/data")];

        commit_upper_dir_to_image_with_volumes(
            Arc::clone(&store),
            &upper_dir,
            "volume-image:latest",
            &config,
            &volumes,
        )
        .expect("commit image");

        let bytes = store
            .load_config_blob_pub("volume-image", "latest")
            .expect("stored config");
        assert_eq!(parse_volume_paths(&bytes).expect("parse volumes"), volumes);
    }
}
