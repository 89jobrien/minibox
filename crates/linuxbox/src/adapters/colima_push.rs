//! Colima image push adapter.
//!
//! Exports a locally stored image to a Docker archive on a host/VM shared
//! path, loads that archive into the Colima VM via the [`ImageLoader`] port,
//! and delegates the registry upload to `nerdctl push` inside the VM.

use super::colima::LimaExecutor;
use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use minibox_core::as_any;
use minibox_core::domain::{
    DynImageLoader, DynImagePusher, ImagePusher, PushProgress, PushResult, RegistryCredentials,
};
use minibox_core::image::ImageStore;
use minibox_core::image::manifest::OciManifest;
use minibox_core::image::reference::ImageRef;
use serde::Deserialize;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

pub struct ColimaImagePusher {
    image_store: Arc<ImageStore>,
    image_loader: DynImageLoader,
    export_dir: PathBuf,
    executor: LimaExecutor,
}

impl ColimaImagePusher {
    pub fn new(
        image_store: Arc<ImageStore>,
        image_loader: DynImageLoader,
        export_dir: PathBuf,
        executor: LimaExecutor,
    ) -> Self {
        Self {
            image_store,
            image_loader,
            export_dir,
            executor,
        }
    }
}

as_any!(ColimaImagePusher);

#[async_trait]
impl ImagePusher for ColimaImagePusher {
    async fn push_image(
        &self,
        image_ref: &ImageRef,
        credentials: &RegistryCredentials,
        progress_tx: Option<mpsc::Sender<PushProgress>>,
    ) -> Result<PushResult> {
        std::fs::create_dir_all(&self.export_dir)
            .with_context(|| format!("create export dir {}", self.export_dir.display()))?;

        let full_ref = format!("{}:{}", image_ref.cache_name(), image_ref.tag);
        let archive_path = self.export_dir.join(format!(
            "{}-{}.tar",
            image_ref.name,
            Uuid::new_v4().simple()
        ));

        let image_store = Arc::clone(&self.image_store);
        let image_ref_for_export = image_ref.clone();
        let archive_path_for_export = archive_path.clone();
        let exported = tokio::task::spawn_blocking(move || {
            export_image_as_docker_archive(
                &image_store,
                &image_ref_for_export,
                &archive_path_for_export,
            )
        })
        .await
        .context("spawn_blocking export image for Colima push")??;

        if let Some(tx) = &progress_tx {
            for layer in &exported.layer_digests {
                let _ = tx
                    .send(PushProgress {
                        layer_digest: layer.clone(),
                        bytes_uploaded: 0,
                        total_bytes: 0,
                    })
                    .await;
            }
        }

        self.image_loader
            .load_image(&archive_path, &image_ref.cache_name(), &image_ref.tag)
            .await
            .with_context(|| format!("load {} into Colima VM", archive_path.display()))?;

        run_colima_push(&self.executor, image_ref, credentials)
            .with_context(|| format!("nerdctl push {full_ref}"))?;

        let digest =
            inspect_pushed_digest(&self.executor, &full_ref).unwrap_or(exported.manifest_digest);

        let _ = std::fs::remove_file(&archive_path);

        if let Some(tx) = &progress_tx {
            for layer in &exported.layer_digests {
                let _ = tx
                    .send(PushProgress {
                        layer_digest: layer.clone(),
                        bytes_uploaded: exported.size_bytes,
                        total_bytes: exported.size_bytes,
                    })
                    .await;
            }
        }

        Ok(PushResult {
            digest,
            size_bytes: exported.size_bytes,
        })
    }
}

#[derive(Debug)]
struct ExportedArchive {
    manifest_digest: String,
    layer_digests: Vec<String>,
    size_bytes: u64,
}

#[derive(Debug, Serialize)]
struct DockerArchiveManifestEntry {
    #[serde(rename = "Config")]
    config: String,
    #[serde(rename = "RepoTags")]
    repo_tags: Vec<String>,
    #[serde(rename = "Layers")]
    layers: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct NerdctlPushInspect {
    #[serde(rename = "RepoDigests")]
    repo_digests: Option<Vec<String>>,
}

fn export_image_as_docker_archive(
    image_store: &ImageStore,
    image_ref: &ImageRef,
    tar_path: &Path,
) -> Result<ExportedArchive> {
    let cache_name = image_ref.cache_name();
    let tag = &image_ref.tag;
    let manifest = image_store
        .load_manifest_pub(&cache_name, tag)
        .with_context(|| format!("load manifest for {cache_name}:{tag}"))?;
    let layers_dir = image_store
        .layers_dir_pub(&cache_name, tag)
        .with_context(|| format!("layers dir for {cache_name}:{tag}"))?;

    let export_parent = tar_path
        .parent()
        .ok_or_else(|| anyhow!("archive path has no parent: {}", tar_path.display()))?;
    std::fs::create_dir_all(export_parent)
        .with_context(|| format!("create export parent {}", export_parent.display()))?;
    let staging = tempfile::tempdir_in(export_parent).context("create staging dir")?;

    let config_bytes = read_or_synthesize_config(&layers_dir, &manifest)?;
    let config_digest = format!("{:x}", Sha256::digest(&config_bytes));
    let config_name = format!("{config_digest}.json");
    std::fs::write(staging.path().join(&config_name), &config_bytes).with_context(|| {
        format!(
            "write config for exported image {}:{}",
            image_ref.cache_name(),
            image_ref.tag
        )
    })?;

    let mut layer_paths = Vec::with_capacity(manifest.layers.len());
    let mut total_size = 0_u64;
    let mut layer_digests = Vec::with_capacity(manifest.layers.len());
    for (idx, layer_desc) in manifest.layers.iter().enumerate() {
        let bytes = layer_archive_bytes(&layers_dir, &layer_desc.digest)
            .with_context(|| format!("read archived bytes for {}", layer_desc.digest))?;
        total_size += bytes.len() as u64;
        layer_digests.push(layer_desc.digest.clone());

        let layer_name = format!("layer-{idx}.tar");
        std::fs::write(staging.path().join(&layer_name), &bytes)
            .with_context(|| format!("write exported layer {layer_name}"))?;
        layer_paths.push(layer_name);
    }

    let full_ref = format!("{}:{}", image_ref.cache_name(), image_ref.tag);
    let docker_manifest = vec![DockerArchiveManifestEntry {
        config: config_name,
        repo_tags: vec![full_ref],
        layers: layer_paths,
    }];
    let docker_manifest_bytes =
        serde_json::to_vec(&docker_manifest).context("serialize docker archive manifest.json")?;
    std::fs::write(staging.path().join("manifest.json"), &docker_manifest_bytes)
        .context("write docker archive manifest.json")?;

    write_archive_from_dir(staging.path(), tar_path)?;

    let manifest_digest = format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&manifest).context("serialize OCI manifest")?)
    );

    Ok(ExportedArchive {
        manifest_digest,
        layer_digests,
        size_bytes: total_size,
    })
}

fn read_or_synthesize_config(layers_dir: &Path, manifest: &OciManifest) -> Result<Vec<u8>> {
    let config_path = layers_dir.join("config.json");
    if config_path.exists() {
        return std::fs::read(&config_path)
            .with_context(|| format!("read {}", config_path.display()));
    }

    serde_json::to_vec(&serde_json::json!({
        "architecture": "amd64",
        "os": "linux",
        "config": {},
        "rootfs": {
            "type": "layers",
            "diff_ids": manifest.layers.iter().map(|layer| layer.digest.clone()).collect::<Vec<_>>(),
        },
    }))
    .context("serialize synthesized config")
}

fn layer_archive_bytes(layers_dir: &Path, digest: &str) -> Result<Vec<u8>> {
    let digest_key = digest.replace(':', "_");
    let raw_layer = layers_dir.join(format!("{digest_key}.tar"));
    if raw_layer.exists() {
        return std::fs::read(&raw_layer)
            .with_context(|| format!("read raw layer blob {}", raw_layer.display()));
    }

    let extracted_layer = layers_dir.join(&digest_key);
    retar_layer_dir(&extracted_layer)
}

fn retar_layer_dir(dir: &Path) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut buf);
        builder.follow_symlinks(false);
        builder
            .append_dir_all(".", dir)
            .with_context(|| format!("append layer dir {}", dir.display()))?;
        builder.into_inner().context("finish tar builder")?;
    }
    Ok(buf)
}

fn write_archive_from_dir(staging_dir: &Path, tar_path: &Path) -> Result<()> {
    let tar_file = std::fs::File::create(tar_path)
        .with_context(|| format!("create archive {}", tar_path.display()))?;
    let mut builder = tar::Builder::new(tar_file);

    let mut entries = std::fs::read_dir(staging_dir)
        .with_context(|| format!("read staging dir {}", staging_dir.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("collect staging dir entries")?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        builder
            .append_path_with_name(&path, Path::new(&name))
            .with_context(|| format!("append {} to {}", path.display(), tar_path.display()))?;
    }

    builder.finish().context("finish docker archive tar")?;
    Ok(())
}

fn run_colima_push(
    executor: &LimaExecutor,
    image_ref: &ImageRef,
    credentials: &RegistryCredentials,
) -> Result<()> {
    let full_ref = format!("{}:{}", image_ref.cache_name(), image_ref.tag);
    match credentials {
        RegistryCredentials::Anonymous => executor(&["nerdctl", "push", &full_ref]).map(|_| ()),
        RegistryCredentials::Basic { username, password } => {
            let registry_host = image_ref.registry_host();
            let script = format!(
                "printf %s {password} | nerdctl login {registry} --username {username} --password-stdin >/dev/null && nerdctl push {image} && nerdctl logout {registry} >/dev/null 2>&1 || true",
                password = shell_single_quote(password),
                registry = shell_single_quote(registry_host),
                username = shell_single_quote(username),
                image = shell_single_quote(&full_ref),
            );
            executor(&["sh", "-lc", &script]).map(|_| ())
        }
        RegistryCredentials::Token(_) => {
            bail!("token-based registry auth is not supported by the Colima image pusher yet")
        }
    }
}

fn inspect_pushed_digest(executor: &LimaExecutor, full_ref: &str) -> Option<String> {
    let output = executor(&["nerdctl", "image", "inspect", full_ref]).ok()?;
    let inspect: Vec<NerdctlPushInspect> = serde_json::from_str(&output).ok()?;
    inspect
        .first()?
        .repo_digests
        .as_ref()?
        .iter()
        .find_map(|repo_digest| {
            repo_digest
                .strip_prefix(&format!("{full_ref}@"))
                .map(str::to_owned)
        })
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

pub fn colima_image_pusher(
    image_store: Arc<ImageStore>,
    image_loader: DynImageLoader,
    export_dir: PathBuf,
    executor: LimaExecutor,
) -> DynImagePusher {
    Arc::new(ColimaImagePusher::new(
        image_store,
        image_loader,
        export_dir,
        executor,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::domain::ImageLoader;
    use minibox_core::image::manifest::Descriptor;
    use std::sync::Mutex;

    struct RecordingLoader {
        loaded_paths: Arc<Mutex<Vec<PathBuf>>>,
    }

    #[async_trait]
    impl ImageLoader for RecordingLoader {
        async fn load_image(&self, path: &Path, _name: &str, _tag: &str) -> Result<()> {
            self.loaded_paths.lock().unwrap().push(path.to_path_buf());
            Ok(())
        }
    }

    fn sample_manifest(digest: &str) -> OciManifest {
        OciManifest {
            schema_version: 2,
            media_type: "application/vnd.oci.image.manifest.v1+json".to_string(),
            config: Descriptor {
                media_type: "application/vnd.oci.image.config.v1+json".to_string(),
                size: 32,
                digest: "sha256:cfg".to_string(),
                platform: None,
            },
            layers: vec![Descriptor {
                media_type: "application/vnd.oci.image.layer.v1.tar".to_string(),
                size: 16,
                digest: digest.to_string(),
                platform: None,
            }],
        }
    }

    fn sample_layer_tar() -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut buf);
            let data = b"hello";
            let mut header = tar::Header::new_gnu();
            header.set_path("hello.txt").unwrap();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, &data[..]).unwrap();
            builder.finish().unwrap();
        }
        buf
    }

    #[test]
    fn export_image_as_docker_archive_writes_manifest_config_and_layers() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ImageStore::new(tmp.path().join("images")).unwrap();
        let digest = "sha256:abc123";
        let manifest = sample_manifest(digest);
        store
            .store_manifest("example/app", "latest", &manifest)
            .unwrap();

        let layers_dir = store.layers_dir_pub("example/app", "latest").unwrap();
        std::fs::create_dir_all(&layers_dir).unwrap();
        std::fs::write(
            layers_dir.join("config.json"),
            br#"{"architecture":"amd64","os":"linux"}"#,
        )
        .unwrap();
        std::fs::write(
            layers_dir.join(format!("{}.tar", digest.replace(':', "_"))),
            sample_layer_tar(),
        )
        .unwrap();

        let image_ref = ImageRef::parse("example/app:latest").unwrap();
        let archive_path = tmp.path().join("export.tar");
        let exported = export_image_as_docker_archive(&store, &image_ref, &archive_path).unwrap();

        assert!(archive_path.exists(), "docker archive should be written");
        assert_eq!(exported.layer_digests, vec![digest.to_string()]);

        let unpacked = tempfile::TempDir::new().unwrap();
        let file = std::fs::File::open(&archive_path).unwrap();
        let mut archive = tar::Archive::new(file);
        archive.unpack(unpacked.path()).unwrap();

        assert!(unpacked.path().join("manifest.json").exists());
        let manifest_json = std::fs::read_to_string(unpacked.path().join("manifest.json")).unwrap();
        assert!(manifest_json.contains("example/app:latest"));
    }

    #[tokio::test]
    async fn colima_image_pusher_loads_archive_then_pushes_with_nerdctl() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(tmp.path().join("images")).unwrap());
        let digest = "sha256:abc123";
        let manifest = sample_manifest(digest);
        store
            .store_manifest("127.0.0.1:5001/example/app", "latest", &manifest)
            .unwrap();
        let layers_dir = store
            .layers_dir_pub("127.0.0.1:5001/example/app", "latest")
            .unwrap();
        std::fs::create_dir_all(&layers_dir).unwrap();
        std::fs::write(
            layers_dir.join("config.json"),
            br#"{"architecture":"amd64","os":"linux"}"#,
        )
        .unwrap();
        std::fs::write(
            layers_dir.join(format!("{}.tar", digest.replace(':', "_"))),
            sample_layer_tar(),
        )
        .unwrap();

        let loaded_paths = Arc::new(Mutex::new(Vec::new()));
        let loader: DynImageLoader = Arc::new(RecordingLoader {
            loaded_paths: Arc::clone(&loaded_paths),
        });
        let commands = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let commands_for_exec = Arc::clone(&commands);
        let executor: LimaExecutor = Arc::new(move |args: &[&str]| {
            commands_for_exec
                .lock()
                .unwrap()
                .push(args.iter().map(|arg| arg.to_string()).collect());
            if args.get(0) == Some(&"nerdctl") && args.get(1) == Some(&"image") {
                return Ok(
                    r#"[{"RepoDigests":["127.0.0.1:5001/example/app:latest@sha256:deadbeef"]}]"#
                        .to_string(),
                );
            }
            Ok(String::new())
        });

        let pusher = ColimaImagePusher::new(
            Arc::clone(&store),
            loader,
            tmp.path().join("exports"),
            executor,
        );

        let result = pusher
            .push_image(
                &ImageRef::parse("127.0.0.1:5001/example/app:latest").unwrap(),
                &RegistryCredentials::Anonymous,
                None,
            )
            .await
            .unwrap();

        assert_eq!(result.digest, "sha256:deadbeef");
        assert_eq!(loaded_paths.lock().unwrap().len(), 1);

        let recorded = commands.lock().unwrap();
        assert!(
            recorded.iter().any(|cmd| cmd
                == &vec![
                    "nerdctl".to_string(),
                    "push".to_string(),
                    "127.0.0.1:5001/example/app:latest".to_string()
                ]),
            "expected nerdctl push command, got {recorded:?}"
        );
    }
}
