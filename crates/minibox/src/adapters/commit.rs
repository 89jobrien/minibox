//! Overlay filesystem commit adapter.
//!
//! Snapshots a container's writable layer (upperdir) into a new OCI image
//! by tarring the upperdir, storing it as a new layer blob, and constructing
//! a new OCI manifest.

use crate::container_state::StateHandle;
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use minibox_core::as_any;
use minibox_core::domain::{
    CommitConfig, ContainerCommitter, ContainerId, DynContainerCommitter, ImageMetadata, LayerInfo,
};
use minibox_core::image::ImageStore;
use minibox_core::image::manifest::{Descriptor, OciManifest};
use minibox_core::image::reference::ImageRef;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    ) -> Result<ImageMetadata> {
        let id = container_id.as_str().to_string();

        let merged_rootfs = self
            .state
            .get_merged_rootfs(&id)
            .await
            .with_context(|| format!("container {id} has no merged rootfs"))?;
        let source_image_ref = self
            .state
            .get_source_image_ref(&id)
            .await
            .with_context(|| format!("container {id} has no source image reference"))?;
        let image_store = Arc::clone(&self.image_store);
        let target_ref = target_ref.to_string();
        let config = config.clone();

        tokio::task::spawn_blocking(move || {
            commit_merged_rootfs_with_base(
                image_store,
                &merged_rootfs,
                &source_image_ref,
                &target_ref,
                &config,
            )
        })
        .await
        .context("spawn_blocking commit")?
    }
}

fn commit_merged_rootfs_with_base(
    image_store: Arc<ImageStore>,
    merged_rootfs: &Path,
    source_ref: &str,
    target_ref: &str,
    commit_config: &CommitConfig,
) -> Result<ImageMetadata> {
    let image_ref = ImageRef::parse(source_ref)
        .with_context(|| format!("parse source image reference {source_ref:?}"))?;
    let name = image_ref.cache_name();
    let tag = image_ref.tag;
    let _manifest = image_store
        .load_manifest_pub(&name, &tag)
        .with_context(|| format!("load source manifest {name}:{tag}"))?;
    let mut config: Value = match image_store.load_config_blob_pub(&name, &tag) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("parse source config {name}:{tag}"))?,
        Err(_) => json!({
            "architecture": match std::env::consts::ARCH {
                "aarch64" => "arm64",
                "x86_64" => "amd64",
                other => other,
            },
            "os": "linux",
            "config": {}
        }),
    };
    apply_commit_config(&mut config, commit_config)?;
    commit_layer_stack_to_image(
        image_store,
        &[LayerInput::existing(merged_rootfs.to_path_buf())],
        target_ref,
        config,
    )
}

fn apply_commit_config(config: &mut Value, commit_config: &CommitConfig) -> Result<()> {
    let root = config
        .as_object_mut()
        .context("OCI source config root must be an object")?;
    if let Some(author) = &commit_config.author {
        root.insert("author".to_string(), json!(author));
    }
    if let Some(message) = &commit_config.message {
        let history = root
            .entry("history")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .context("OCI history must be an array")?;
        history.push(json!({"comment": message}));
    }
    let image_config = root
        .entry("config")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("OCI config.config must be an object")?;
    let env = image_config
        .entry("Env")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .context("OCI config Env must be an array")?;
    for override_entry in &commit_config.env_overrides {
        let key = override_entry
            .split_once('=')
            .map_or(override_entry.as_str(), |(key, _)| key);
        let prefix = format!("{key}=");
        if let Some(existing) = env.iter_mut().find(|entry| {
            entry
                .as_str()
                .is_some_and(|entry| entry.starts_with(&prefix))
        }) {
            *existing = json!(override_entry);
        } else {
            env.push(json!(override_entry));
        }
    }
    if let Some(command) = &commit_config.cmd_override {
        image_config.insert("Cmd".to_string(), json!(command));
    }
    Ok(())
}

/// Packages an overlay upper directory as a new local image.
pub fn commit_upper_dir_to_image(
    image_store: Arc<ImageStore>,
    upper_dir: &std::path::Path,
    target_ref: &str,
    config: &CommitConfig,
) -> Result<ImageMetadata> {
    let config_json = json!({
        "architecture": "amd64",
        "os": "linux",
        "author": config.author.clone(),
        "history": config.message.as_ref().map(|message| vec![json!({"comment": message})]).unwrap_or_default(),
        "config": {
            "Env": config.env_overrides.clone(),
            "Cmd": config.cmd_override.clone().unwrap_or_default(),
        }
    });
    commit_layer_stack_to_image(
        image_store,
        &[LayerInput::generated(upper_dir.to_path_buf())],
        target_ref,
        config_json,
    )
}

/// One ordered rootfs layer used while assembling an image build result.
#[derive(Debug, Clone)]
pub(crate) struct LayerInput {
    pub(crate) directory: PathBuf,
}

impl LayerInput {
    pub(crate) const fn generated(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub(crate) const fn existing(directory: PathBuf) -> Self {
        Self { directory }
    }
}

/// Flatten an ordered layer stack and persist one self-contained OCI layer.
///
/// Flattening avoids retaining registry descriptors when their original raw
/// blobs are unavailable locally and gives correct whiteout semantics by
/// materializing the merged filesystem before creating the final tar stream.
pub(crate) fn commit_layer_stack_to_image(
    image_store: Arc<ImageStore>,
    layers: &[LayerInput],
    target_ref: &str,
    mut config_json: Value,
) -> Result<ImageMetadata> {
    let (target_name, target_tag) = parse_image_ref(target_ref);
    let flattened = tempfile::TempDir::new().context("create flattened image rootfs")?;
    materialize_layer_directories(layers, flattened.path())?;
    let stored = store_snapshot_layer(&image_store, flattened.path(), &target_name, &target_tag)?;
    let descriptor = stored.descriptor;
    let diff_id = stored.diff_id;

    set_rootfs_diff_ids(&mut config_json, vec![diff_id])?;
    use sha2::{Digest, Sha256};
    let config_bytes = serde_json::to_vec(&config_json).context("serialize image config")?;
    let config_digest = format!("sha256:{:x}", Sha256::digest(&config_bytes));
    image_store
        .store_config_blob(&target_name, &target_tag, &config_bytes)
        .context("store image config")?;

    let manifest = OciManifest {
        schema_version: 2,
        media_type: "application/vnd.oci.image.manifest.v1+json".to_string(),
        config: Descriptor {
            media_type: "application/vnd.oci.image.config.v1+json".to_string(),
            size: config_bytes.len() as u64,
            digest: config_digest,
            platform: None,
        },
        layers: vec![descriptor.clone()],
    };
    image_store
        .store_manifest(&target_name, &target_tag, &manifest)
        .context("store new manifest")?;

    Ok(ImageMetadata {
        name: target_name,
        tag: target_tag,
        layers: vec![LayerInfo {
            digest: descriptor.digest,
            size: descriptor.size,
        }],
    })
}

#[derive(Debug, Clone)]
pub(crate) struct StoredSnapshotLayer {
    pub(crate) layer: LayerInput,
    descriptor: Descriptor,
    diff_id: String,
}

/// Store a flattened rootfs as a durable image-store-owned layer.
pub(crate) fn store_snapshot_layer(
    image_store: &ImageStore,
    rootfs: &Path,
    name: &str,
    tag: &str,
) -> Result<StoredSnapshotLayer> {
    use flate2::{Compression, GzBuilder};
    use sha2::{Digest, Sha256};
    use std::io::{Cursor, Write};

    let tar_bytes = tar_directory(rootfs)?;
    let diff_id = format!("sha256:{:x}", Sha256::digest(&tar_bytes));
    let mut encoder = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::default());
    encoder
        .write_all(&tar_bytes)
        .context("compress layer tar")?;
    let blob = encoder.finish().context("finish layer compression")?;
    let digest = format!("sha256:{:x}", Sha256::digest(&blob));
    let directory = image_store
        .store_layer_verified(name, tag, &digest, Cursor::new(&blob))
        .context("store durable snapshot layer")?;
    let layers_dir = image_store.layers_dir_pub(name, tag)?;
    let digest_key = digest.replace(':', "_");
    std::fs::write(layers_dir.join(format!("{digest_key}.tar")), &blob)
        .context("store matching raw layer blob")?;
    Ok(StoredSnapshotLayer {
        layer: LayerInput::existing(directory),
        descriptor: Descriptor {
            media_type: "application/vnd.oci.image.layer.v1.tar+gzip".to_string(),
            size: blob.len() as u64,
            digest,
            platform: None,
        },
        diff_id,
    })
}

fn set_rootfs_diff_ids(config: &mut Value, diff_ids: Vec<String>) -> Result<()> {
    let object = config
        .as_object_mut()
        .context("OCI config root must be an object")?;
    object.insert(
        "rootfs".to_string(),
        json!({"type": "layers", "diff_ids": diff_ids}),
    );
    object
        .entry("config")
        .or_insert_with(|| Value::Object(Map::new()));
    Ok(())
}

/// Materialize ordered OCI layer directories into one merged rootfs.
pub(crate) fn materialize_layer_directories(
    layers: &[LayerInput],
    destination: &Path,
) -> Result<()> {
    remove_path_if_exists(destination)?;
    std::fs::create_dir_all(destination)
        .with_context(|| format!("create materialized rootfs {}", destination.display()))?;
    for layer in layers {
        apply_layer_directory(&layer.directory, destination)?;
    }
    Ok(())
}

fn apply_layer_directory(source: &Path, destination: &Path) -> Result<()> {
    let destination_metadata = std::fs::symlink_metadata(destination)
        .with_context(|| format!("inspect materialization root {}", destination.display()))?;
    if destination_metadata.file_type().is_symlink() || !destination_metadata.is_dir() {
        bail!(
            "materialization destination is not a real directory: {}",
            destination.display()
        );
    }
    for entry in std::fs::read_dir(source)
        .with_context(|| format!("read layer directory {}", source.display()))?
    {
        let entry = entry.context("read layer entry")?;
        let name = entry.file_name();
        let name_lossy = name.to_string_lossy();
        if name_lossy == ".wh..wh..opq" {
            clear_directory(destination)?;
            continue;
        }
        if let Some(removed) = name_lossy.strip_prefix(".wh.") {
            remove_path_if_exists(&destination.join(removed))?;
            continue;
        }

        let source_path = entry.path();
        let target = destination.join(&name);
        let metadata = std::fs::symlink_metadata(&source_path)
            .with_context(|| format!("inspect layer entry {}", source_path.display()))?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            if let Ok(target_metadata) = std::fs::symlink_metadata(&target)
                && (target_metadata.file_type().is_symlink() || !target_metadata.is_dir())
            {
                remove_path_if_exists(&target)?;
            }
            std::fs::create_dir_all(&target)
                .with_context(|| format!("create merged directory {}", target.display()))?;
            apply_layer_directory(&source_path, &target)?;
            std::fs::set_permissions(&target, metadata.permissions())?;
            preserve_ownership(&target, &metadata)?;
        } else {
            remove_path_if_exists(&target)?;
            if metadata.file_type().is_symlink() {
                copy_symlink(&source_path, &target)?;
                preserve_ownership(&target, &metadata)?;
            } else if metadata.is_file() {
                std::fs::copy(&source_path, &target)?;
                std::fs::set_permissions(&target, metadata.permissions())?;
                preserve_ownership(&target, &metadata)?;
            } else {
                bail!("unsupported layer entry type: {}", source_path.display());
            }
        }
    }
    Ok(())
}

fn clear_directory(directory: &Path) -> Result<()> {
    for entry in std::fs::read_dir(directory)? {
        remove_path_if_exists(&entry?.path())?;
    }
    Ok(())
}

fn remove_path_if_exists(path: &Path) -> Result<()> {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return Ok(());
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
    .with_context(|| format!("remove existing path {}", path.display()))
}

#[cfg(unix)]
fn preserve_ownership(path: &Path, metadata: &std::fs::Metadata) -> Result<()> {
    use nix::fcntl::AtFlags;
    use nix::unistd::{Gid, Uid, fchownat};
    use std::os::unix::fs::MetadataExt;

    fchownat(
        None,
        path,
        Some(Uid::from_raw(metadata.uid())),
        Some(Gid::from_raw(metadata.gid())),
        AtFlags::AT_SYMLINK_NOFOLLOW,
    )
    .with_context(|| format!("preserve layer ownership for {}", path.display()))
}

#[cfg(not(unix))]
fn preserve_ownership(_path: &Path, _metadata: &std::fs::Metadata) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(source: &Path, destination: &Path) -> Result<()> {
    let target =
        std::fs::read_link(source).with_context(|| format!("read symlink {}", source.display()))?;
    std::os::unix::fs::symlink(&target, destination)
        .with_context(|| format!("copy symlink {}", source.display()))
}

#[cfg(not(unix))]
fn copy_symlink(source: &Path, _destination: &Path) -> Result<()> {
    bail!(
        "preserving symlinks while committing images is unsupported on this platform: {}",
        source.display()
    )
}

fn tar_directory(dir: &std::path::Path) -> Result<Vec<u8>> {
    use tar::Builder;
    let mut buf = Vec::new();
    {
        let mut ar = Builder::new(&mut buf);
        // See docker_archive::tar_directory for why this must be false:
        // layers commonly contain symlinks (e.g. etc/mtab -> ../proc/mounts)
        // that only resolve inside a live container mount namespace and
        // would otherwise fail ENOENT when append_dir_all follows them.
        ar.follow_symlinks(false);
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
        let bytes = tar_directory(tmp.path()).unwrap();
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
        let bytes = tar_directory(tmp.path()).expect("tar despite dangling symlink");
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
            Arc::clone(&image_store),
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

        let stored_layers = image_store
            .get_image_layers("myapp", "v1")
            .expect("load committed layer paths");
        assert_eq!(stored_layers.len(), 1);
        assert!(
            stored_layers[0].join("hello.txt").is_file(),
            "committed layer must be usable as an extracted rootfs layer"
        );
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

    #[test]
    fn container_commit_preserves_base_layers_and_config() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let store = Arc::new(
            minibox_core::image::ImageStore::new(temp.path().join("images"))
                .expect("create image store"),
        );
        let base_layers = store
            .layers_dir_pub("library/base", "latest")
            .expect("base layers dir");
        let base_layer = base_layers.join("sha256_base");
        std::fs::create_dir_all(&base_layer).expect("create base layer");
        std::fs::write(base_layer.join("base.txt"), "base").expect("write base file");
        store
            .store_config_blob(
                "library/base",
                "latest",
                br#"{"architecture":"amd64","os":"linux","config":{"Env":["BASE=1"],"Entrypoint":["/base"]},"rootfs":{"type":"layers","diff_ids":["sha256:base-diff"]}}"#,
            )
            .expect("store base config");
        store
            .store_manifest(
                "library/base",
                "latest",
                &OciManifest {
                    schema_version: 2,
                    media_type: "application/vnd.oci.image.manifest.v1+json".to_string(),
                    config: Descriptor {
                        media_type: "application/vnd.oci.image.config.v1+json".to_string(),
                        size: 0,
                        digest: "sha256:config".to_string(),
                        platform: None,
                    },
                    layers: vec![Descriptor {
                        media_type: "application/vnd.oci.image.layer.v1.tar".to_string(),
                        size: 4,
                        digest: "sha256:base".to_string(),
                        platform: None,
                    }],
                },
            )
            .expect("store base manifest");
        let merged = temp.path().join("merged");
        std::fs::create_dir(&merged).expect("create merged rootfs");
        std::fs::write(merged.join("base.txt"), "base").expect("write inherited file");
        std::fs::write(merged.join("change.txt"), "change").expect("write changed file");

        commit_merged_rootfs_with_base(
            Arc::clone(&store),
            &merged,
            "base:latest",
            "committed:latest",
            &CommitConfig {
                author: Some("tester".to_string()),
                message: Some("commit".to_string()),
                env_overrides: vec!["BASE=2".to_string(), "NEW=1".to_string()],
                cmd_override: Some(vec!["run".to_string()]),
            },
        )
        .expect("commit with base");

        let manifest = store
            .load_manifest_pub("committed", "latest")
            .expect("load committed manifest");
        assert_eq!(manifest.layers.len(), 1);
        let committed_layers = store
            .get_image_layers("committed", "latest")
            .expect("load committed layers");
        assert_eq!(
            std::fs::read_to_string(committed_layers[0].join("base.txt"))
                .expect("read inherited file"),
            "base"
        );
        assert_eq!(
            std::fs::read_to_string(committed_layers[0].join("change.txt"))
                .expect("read changed file"),
            "change"
        );
        let config: Value = serde_json::from_slice(
            &store
                .load_config_blob_pub("committed", "latest")
                .expect("load committed config"),
        )
        .expect("parse committed config");
        assert_eq!(config["config"]["Entrypoint"][0], "/base");
        assert_eq!(config["config"]["Env"], json!(["BASE=2", "NEW=1"]));
        assert_eq!(config["config"]["Cmd"], json!(["run"]));
    }

    #[test]
    fn flattened_commit_applies_overlay_whiteouts() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let store = Arc::new(ImageStore::new(temp.path().join("images")).expect("create store"));
        let base = temp.path().join("base");
        let upper = temp.path().join("upper");
        std::fs::create_dir(&base).expect("create base");
        std::fs::create_dir(&upper).expect("create upper");
        std::fs::write(base.join("removed.txt"), "old").expect("write base file");
        std::fs::write(upper.join(".wh.removed.txt"), "").expect("write whiteout");
        std::fs::write(upper.join("kept.txt"), "new").expect("write upper file");

        commit_layer_stack_to_image(
            Arc::clone(&store),
            &[LayerInput::existing(base), LayerInput::generated(upper)],
            "whiteout:test",
            json!({"architecture": "amd64", "os": "linux", "config": {}}),
        )
        .expect("commit flattened whiteout image");

        let layers = store
            .get_image_layers("whiteout", "test")
            .expect("load whiteout image");
        assert_eq!(layers.len(), 1);
        assert!(!layers[0].join("removed.txt").exists());
        assert_eq!(
            std::fs::read_to_string(layers[0].join("kept.txt")).expect("read kept file"),
            "new"
        );
        assert!(!layers[0].join(".wh.removed.txt").exists());
    }
}
