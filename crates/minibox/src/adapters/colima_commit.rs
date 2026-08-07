//! Colima container commit adapter.
//!
//! Implements [`ContainerCommitter`] for the Colima (Lima) adapter suite by
//! shelling out to `nerdctl commit` and `nerdctl save` inside the Lima VM,
//! then parsing the resulting Docker-archive tarball and importing it into
//! the local [`ImageStore`] — mirroring, in reverse, the docker-archive
//! export logic in `colima_push.rs`.

use super::colima::LimaExecutor;
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use minibox_core::as_any;
use minibox_core::domain::{
    CommitConfig, ContainerCommitter, ContainerId, DynContainerCommitter, ImageMetadata, LayerInfo,
};
use minibox_core::image::ImageStore;
use minibox_core::image::manifest::{Descriptor, OciManifest};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

/// Colima implementation of [`ContainerCommitter`].
///
/// Commits a running/stopped container to a new image via `nerdctl commit`,
/// exports it via `nerdctl save`, and imports the resulting Docker-archive
/// tarball into the shared [`ImageStore`] so it is visible to the rest of
/// minibox (`mbx run`, `mbx images`, push, etc.) the same way a natively
/// pulled image would be.
pub struct ColimaContainerCommitter {
    image_store: Arc<ImageStore>,
    export_dir: PathBuf,
    executor: LimaExecutor,
}

impl ColimaContainerCommitter {
    pub fn new(image_store: Arc<ImageStore>, export_dir: PathBuf, executor: LimaExecutor) -> Self {
        Self {
            image_store,
            export_dir,
            executor,
        }
    }
}

as_any!(ColimaContainerCommitter);

#[async_trait]
impl ContainerCommitter for ColimaContainerCommitter {
    // qual:allow(iosp) reason: "adapter I/O boundary — nerdctl commit/save + tarball import"
    async fn commit(
        &self,
        container_id: &ContainerId,
        target_ref: &str,
        config: &CommitConfig,
    ) -> Result<ImageMetadata> {
        std::fs::create_dir_all(&self.export_dir)
            .with_context(|| format!("create export dir {}", self.export_dir.display()))?;

        let (target_name, target_tag) = parse_image_ref(target_ref);
        let full_ref = format!("{target_name}:{target_tag}");

        run_nerdctl_commit(&self.executor, container_id.as_str(), &full_ref, config)
            .with_context(|| format!("nerdctl commit {} -> {full_ref}", container_id.as_str()))?;

        let archive_path = self.export_dir.join(format!(
            "{}-{}.tar",
            target_name.replace('/', "-"),
            Uuid::new_v4().simple()
        ));

        run_nerdctl_save(&self.executor, &full_ref, &archive_path)
            .with_context(|| format!("nerdctl save {full_ref}"))?;

        let image_store = Arc::clone(&self.image_store);
        let archive_path_for_import = archive_path.clone();
        let target_name_for_import = target_name.clone();
        let target_tag_for_import = target_tag.clone();
        let metadata = tokio::task::spawn_blocking(move || {
            import_docker_archive(
                &image_store,
                &archive_path_for_import,
                &target_name_for_import,
                &target_tag_for_import,
            )
        })
        .await
        .context("spawn_blocking import commit archive")??;

        let _ = std::fs::remove_file(&archive_path);

        Ok(metadata)
    }
}

fn run_nerdctl_commit(
    executor: &LimaExecutor,
    container: &str,
    full_ref: &str,
    config: &CommitConfig,
) -> Result<()> {
    let mut args: Vec<String> = vec!["nerdctl".to_string(), "commit".to_string()];
    if let Some(author) = &config.author {
        args.push("--author".to_string());
        args.push(author.clone());
    }
    if let Some(message) = &config.message {
        args.push("--message".to_string());
        args.push(message.clone());
    }
    args.push(container.to_string());
    args.push(full_ref.to_string());

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    executor(&arg_refs).map(|_| ())
}

fn run_nerdctl_save(
    executor: &LimaExecutor,
    full_ref: &str,
    archive_path: &std::path::Path,
) -> Result<()> {
    let archive_str = archive_path
        .to_str()
        .ok_or_else(|| anyhow!("non-UTF-8 archive path: {}", archive_path.display()))?;
    executor(&["nerdctl", "save", full_ref, "-o", archive_str]).map(|_| ())
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct DockerArchiveManifestEntry {
    #[serde(rename = "Config")]
    config: String,
    #[serde(rename = "Layers")]
    layers: Vec<String>,
}

/// Unpack a `nerdctl save` (Docker-archive format) tarball and import its
/// config + layers into `image_store` under `name:tag`, returning the
/// resulting [`ImageMetadata`].
fn import_docker_archive(
    image_store: &ImageStore,
    archive_path: &std::path::Path,
    name: &str,
    tag: &str,
) -> Result<ImageMetadata> {
    let file = std::fs::File::open(archive_path)
        .with_context(|| format!("open commit archive {}", archive_path.display()))?;
    let mut outer = tar::Archive::new(file);
    let tmp = tempfile::TempDir::new().context("create temp dir for commit archive")?;
    outer
        .unpack(tmp.path())
        .with_context(|| format!("unpack commit archive {}", archive_path.display()))?;

    let manifest_path = tmp.path().join("manifest.json");
    let manifest_bytes = std::fs::read(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let docker_manifest: Vec<DockerArchiveManifestEntry> =
        serde_json::from_slice(&manifest_bytes).context("parse docker archive manifest.json")?;
    let entry = docker_manifest
        .first()
        .ok_or_else(|| anyhow!("commit archive manifest.json has no entries"))?;

    let config_path = tmp.path().join(&entry.config);
    let config_bytes = std::fs::read(&config_path)
        .with_context(|| format!("read config {}", config_path.display()))?;
    let config_digest = format!("sha256:{:x}", Sha256::digest(&config_bytes));

    let mut layer_descriptors = Vec::with_capacity(entry.layers.len());
    let mut layer_infos = Vec::with_capacity(entry.layers.len());
    for layer_rel_path in &entry.layers {
        let layer_path = tmp.path().join(layer_rel_path);
        let layer_bytes = std::fs::read(&layer_path)
            .with_context(|| format!("read layer {}", layer_path.display()))?;

        // `nerdctl save` writes each layer as a plain (uncompressed) tar, but
        // ImageStore::store_layer (like a real OCI registry blob) expects a
        // gzip-compressed tar stream and digests the compressed bytes — gzip
        // it here so the digest and stored blob match what extract_layer
        // expects on the read path.
        let compressed = gzip_compress(&layer_bytes)?;
        let layer_digest = format!("sha256:{:x}", Sha256::digest(&compressed));
        let size = compressed.len() as u64;

        image_store
            .store_layer(name, tag, &layer_digest, compressed.as_slice())
            .with_context(|| format!("store layer {layer_digest}"))?;

        layer_descriptors.push(Descriptor {
            media_type: "application/vnd.oci.image.layer.v1.tar+gzip".to_string(),
            size,
            digest: layer_digest.clone(),
            platform: None,
        });
        layer_infos.push(LayerInfo {
            digest: layer_digest,
            size,
        });
    }

    let manifest = OciManifest {
        schema_version: 2,
        media_type: "application/vnd.oci.image.manifest.v1+json".to_string(),
        config: Descriptor {
            media_type: "application/vnd.oci.image.config.v1+json".to_string(),
            size: config_bytes.len() as u64,
            digest: config_digest,
            platform: None,
        },
        layers: layer_descriptors,
    };

    image_store
        .store_manifest(name, tag, &manifest)
        .with_context(|| format!("store manifest for {name}:{tag}"))?;

    Ok(ImageMetadata {
        name: name.to_string(),
        tag: tag.to_string(),
        layers: layer_infos,
    })
}

fn gzip_compress(bytes: &[u8]) -> Result<Vec<u8>> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(bytes)
        .context("write bytes to gzip encoder")?;
    encoder.finish().context("finish gzip encoding")
}

fn parse_image_ref(s: &str) -> (String, String) {
    if let Some((name, tag)) = s.rsplit_once(':') {
        (name.to_string(), tag.to_string())
    } else {
        (s.to_string(), "latest".to_string())
    }
}

pub fn colima_container_committer(
    image_store: Arc<ImageStore>,
    export_dir: PathBuf,
    executor: LimaExecutor,
) -> DynContainerCommitter {
    Arc::new(ColimaContainerCommitter::new(
        image_store,
        export_dir,
        executor,
    ))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn sample_config() -> CommitConfig {
        CommitConfig {
            author: Some("test-author".to_string()),
            message: Some("test-message".to_string()),
            env_overrides: vec![],
            cmd_override: None,
        }
    }

    fn write_layer_tar(dir: &std::path::Path, name: &str, contents: &[u8]) {
        let mut buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut buf);
            let mut header = tar::Header::new_gnu();
            header.set_path("hello.txt").unwrap();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, contents).unwrap();
            builder.finish().unwrap();
        }
        std::fs::write(dir.join(name), buf).unwrap();
    }

    /// Build a fake `nerdctl save` Docker-archive tarball at `out_path`.
    fn write_fake_docker_archive(out_path: &std::path::Path) {
        let staging = tempfile::TempDir::new().unwrap();

        let config_bytes = br#"{"architecture":"amd64","os":"linux","config":{}}"#;
        std::fs::write(staging.path().join("config.json"), config_bytes).unwrap();

        write_layer_tar(staging.path(), "layer-0.tar", b"hello from layer 0");

        let manifest = vec![DockerArchiveManifestEntry {
            config: "config.json".to_string(),
            layers: vec!["layer-0.tar".to_string()],
        }];
        std::fs::write(
            staging.path().join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let tar_file = std::fs::File::create(out_path).unwrap();
        let mut builder = tar::Builder::new(tar_file);
        let mut entries: Vec<_> = std::fs::read_dir(staging.path())
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name();
            builder
                .append_path_with_name(&path, std::path::Path::new(&name))
                .unwrap();
        }
        builder.finish().unwrap();
    }

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
    fn import_docker_archive_stores_manifest_and_layer() {
        let tmp = tempfile::TempDir::new().unwrap();
        let archive_path = tmp.path().join("commit.tar");
        write_fake_docker_archive(&archive_path);

        let image_store = ImageStore::new(tmp.path().join("images")).expect("create image store");

        let metadata = import_docker_archive(&image_store, &archive_path, "example/app", "v1")
            .expect("import commit archive");

        assert_eq!(metadata.name, "example/app");
        assert_eq!(metadata.tag, "v1");
        assert_eq!(metadata.layers.len(), 1);
        assert!(metadata.layers[0].digest.starts_with("sha256:"));
        assert!(metadata.layers[0].size > 0);

        let stored_manifest = image_store
            .load_manifest_pub("example/app", "v1")
            .expect("load stored manifest");
        assert_eq!(stored_manifest.layers.len(), 1);
        assert_eq!(stored_manifest.layers[0].digest, metadata.layers[0].digest);
    }

    #[tokio::test]
    async fn commit_runs_commit_save_and_imports_into_store() {
        let tmp = tempfile::TempDir::new().unwrap();
        let image_store = Arc::new(ImageStore::new(tmp.path().join("images")).unwrap());
        let export_dir = tmp.path().join("exports");

        let commands = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let commands_for_exec = Arc::clone(&commands);
        let executor: LimaExecutor = Arc::new(move |args: &[&str]| {
            let owned: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
            commands_for_exec.lock().unwrap().push(owned.clone());

            if owned.first().map(String::as_str) == Some("nerdctl")
                && owned.get(1).map(String::as_str) == Some("save")
            {
                // The -o argument holds the destination path.
                let out_index = owned
                    .iter()
                    .position(|a| a == "-o")
                    .expect("save command must include -o")
                    + 1;
                let out_path = std::path::PathBuf::from(&owned[out_index]);
                write_fake_docker_archive(&out_path);
            }
            Ok(String::new())
        });

        let committer =
            ColimaContainerCommitter::new(Arc::clone(&image_store), export_dir, executor);

        let container_id = ContainerId::new("abc123".to_string()).expect("valid container id");
        let metadata = committer
            .commit(&container_id, "example/app:latest", &sample_config())
            .await
            .expect("commit should succeed");

        assert_eq!(metadata.name, "example/app");
        assert_eq!(metadata.tag, "latest");
        assert_eq!(metadata.layers.len(), 1);

        let recorded = commands.lock().unwrap();
        assert!(
            recorded
                .iter()
                .any(|cmd| cmd.first().map(String::as_str) == Some("nerdctl")
                    && cmd.get(1).map(String::as_str) == Some("commit")
                    && cmd.contains(&"abc123".to_string())
                    && cmd.contains(&"example/app:latest".to_string())),
            "expected nerdctl commit command, got {recorded:?}"
        );
        assert!(
            recorded
                .iter()
                .any(|cmd| cmd.first().map(String::as_str) == Some("nerdctl")
                    && cmd.get(1).map(String::as_str) == Some("save")
                    && cmd.contains(&"example/app:latest".to_string())),
            "expected nerdctl save command, got {recorded:?}"
        );

        let stored_manifest = image_store
            .load_manifest_pub("example/app", "latest")
            .expect("load stored manifest");
        assert_eq!(stored_manifest.layers.len(), 1);
    }
}
