//! Builds a `docker load`-compatible tarball from a locally pulled OCI image.
//!
//! Used by adapters that run containers via an in-VM `docker` daemon (e.g.
//! the smolvm adapter): the image is pulled once via the host-side
//! [`DockerHubRegistry`](crate::adapters::registry::DockerHubRegistry) (real
//! network access, no VM round-trip), then packaged here into a tar bundle
//! that `docker load` accepts, and imported into the VM via the adapter's
//! existing `ImageLoader::load_image` path.
//!
//! The bundle re-derives `rootfs.diff_ids` from our own re-tarred layer
//! bytes rather than reusing the registry's original layer digests — the
//! extracted-layer-directory representation on disk is not byte-identical to
//! the original compressed blob, so the original `diff_ids` would not
//! validate against a re-tar. `docker load` only requires internal
//! consistency between the config and the layers within the same bundle, so
//! recomputing `diff_ids` over our own re-tar is sufficient; all other config
//! fields (Cmd, Entrypoint, Env, etc.) are preserved unmodified from the
//! real registry config.

use anyhow::{Context, Result, bail};
use minibox_core::image::ImageStore;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Build a docker-load tarball for `name:tag` from `store` into `dest_dir`,
/// returning the tarball path.
///
/// # Errors
///
/// Returns an error if the image's manifest, layers, or config blob are not
/// present in `store`, or if the tarball cannot be written to `dest_dir`.
pub fn build_docker_load_tarball(
    store: &ImageStore,
    name: &str,
    tag: &str,
    dest_dir: &Path,
) -> Result<PathBuf> {
    let manifest = store
        .load_manifest_pub(name, tag)
        .with_context(|| format!("load manifest for {name}:{tag}"))?;
    let layer_dirs = store
        .get_image_layers(name, tag)
        .with_context(|| format!("resolve layer dirs for {name}:{tag}"))?;
    if layer_dirs.len() != manifest.layers.len() {
        bail!(
            "layer count mismatch for {name}:{tag}: manifest declares {}, store has {}",
            manifest.layers.len(),
            layer_dirs.len()
        );
    }

    let config_bytes = store
        .load_config_blob_pub(name, tag)
        .with_context(|| format!("load config blob for {name}:{tag}"))?;
    let mut config_json: serde_json::Value =
        serde_json::from_slice(&config_bytes).context("parse image config JSON")?;

    std::fs::create_dir_all(dest_dir)
        .with_context(|| format!("create export dir {}", dest_dir.display()))?;

    // Re-tar each extracted layer directory to a plain (uncompressed) tar and
    // record its sha256 as the diff_id docker load expects to see in the
    // rewritten config below.
    let mut layer_entries: Vec<(String, Vec<u8>)> = Vec::with_capacity(layer_dirs.len());
    let mut diff_ids: Vec<String> = Vec::with_capacity(layer_dirs.len());
    for (idx, layer_dir) in layer_dirs.iter().enumerate() {
        let tar_bytes = tar_directory(layer_dir)
            .with_context(|| format!("re-tar layer {}", layer_dir.display()))?;
        let digest = format!("sha256:{:x}", Sha256::digest(&tar_bytes));
        diff_ids.push(digest);
        layer_entries.push((format!("layer-{idx}/layer.tar"), tar_bytes));
    }

    config_json["rootfs"] = serde_json::json!({ "type": "layers", "diff_ids": diff_ids });
    let config_out =
        serde_json::to_vec(&config_json).context("serialize rewritten image config")?;

    let repo_tag = format!("{name}:{tag}");
    let docker_manifest = serde_json::json!([{
        "Config": "config.json",
        "RepoTags": [repo_tag],
        "Layers": layer_entries.iter().map(|(path, _)| path.clone()).collect::<Vec<_>>(),
    }]);
    let manifest_out =
        serde_json::to_vec(&docker_manifest).context("serialize docker manifest.json")?;

    let out_path = dest_dir.join(format!("{}.tar", name.replace('/', "_")));
    let file = std::fs::File::create(&out_path)
        .with_context(|| format!("create {}", out_path.display()))?;
    let mut builder = tar::Builder::new(file);

    append_bytes(&mut builder, "config.json", &config_out)?;
    append_bytes(&mut builder, "manifest.json", &manifest_out)?;
    for (path, bytes) in &layer_entries {
        append_bytes(&mut builder, path, bytes)?;
    }
    builder.finish().context("finalize docker-load tarball")?;

    Ok(out_path)
}

fn append_bytes<W: Write>(builder: &mut tar::Builder<W>, path: &str, bytes: &[u8]) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, path, bytes)
        .with_context(|| format!("append {path} to docker-load tarball"))
}

/// Tar up a directory's contents (no compression) and return the bytes.
fn tar_directory(dir: &Path) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut buf);
        // Root filesystem layers commonly contain symlinks that only resolve
        // inside a live container mount namespace (e.g. `etc/mtab ->
        // ../proc/mounts`). `append_dir_all` follows symlinks by default and
        // would fail with ENOENT stat-ing such a target on the host; storing
        // the symlink itself instead avoids that and matches OCI semantics.
        builder.follow_symlinks(false);
        builder
            .append_dir_all(".", dir)
            .with_context(|| format!("append_dir_all {}", dir.display()))?;
        builder.finish().context("finalize layer tar")?;
    }
    Ok(buf)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use minibox_core::image::manifest::{Descriptor, OciManifest};
    use std::sync::Arc;

    fn write_sample_image(store: &ImageStore, name: &str, tag: &str) {
        let manifest = OciManifest {
            schema_version: 2,
            media_type: "application/vnd.oci.image.manifest.v1+json".to_string(),
            config: Descriptor {
                media_type: "application/vnd.oci.image.config.v1+json".to_string(),
                size: 2,
                digest: "sha256:config".to_string(),
                platform: None,
            },
            layers: vec![Descriptor {
                media_type: "application/vnd.oci.image.layer.v1.tar+gzip".to_string(),
                size: 0,
                digest: "sha256:deadbeef".to_string(),
                platform: None,
            }],
        };
        store
            .store_manifest(name, tag, &manifest)
            .expect("store manifest");
        store
            .store_config_blob(name, tag, br#"{"architecture":"amd64","os":"linux"}"#)
            .expect("store config blob");

        let layer_dir = store
            .layers_dir_pub(name, tag)
            .expect("layers dir")
            .join("sha256_deadbeef");
        std::fs::create_dir_all(&layer_dir).expect("create layer dir");
        std::fs::write(layer_dir.join("hello.txt"), b"hi").expect("write layer file");
    }

    #[test]
    fn builds_tarball_with_expected_entries() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(ImageStore::new(tmp.path().join("images")).expect("ImageStore::new"));
        write_sample_image(&store, "alpine", "latest");

        let dest = tmp.path().join("export");
        let tarball =
            build_docker_load_tarball(&store, "alpine", "latest", &dest).expect("build tarball");
        assert!(tarball.exists());

        let file = std::fs::File::open(&tarball).expect("open tarball");
        let mut archive = tar::Archive::new(file);
        let names: Vec<String> = archive
            .entries()
            .expect("entries")
            .map(|e| {
                e.expect("entry")
                    .path()
                    .expect("path")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(names.contains(&"config.json".to_string()));
        assert!(names.contains(&"manifest.json".to_string()));
        assert!(names.iter().any(|n| n.starts_with("layer-0/")));
    }

    /// Regression test: root filesystem layers commonly contain symlinks
    /// that only resolve inside a live container mount namespace, e.g.
    /// Alpine's `etc/mtab -> ../proc/mounts`. `tar_directory` must store
    /// such symlinks as-is rather than following them, or `append_dir_all`
    /// fails with ENOENT trying to stat the (host-side) dangling target.
    #[cfg(unix)]
    #[test]
    fn tar_directory_preserves_dangling_symlinks() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(ImageStore::new(tmp.path().join("images")).expect("ImageStore::new"));
        write_sample_image(&store, "alpine", "latest");

        let layer_dir = store
            .layers_dir_pub("alpine", "latest")
            .expect("layers dir")
            .join("sha256_deadbeef");
        std::os::unix::fs::symlink("../proc/mounts", layer_dir.join("mtab"))
            .expect("create dangling symlink");

        let dest = tmp.path().join("export");
        let tarball = build_docker_load_tarball(&store, "alpine", "latest", &dest)
            .expect("build tarball despite dangling symlink");
        assert!(tarball.exists());
    }

    #[test]
    fn rewrites_config_rootfs_diff_ids() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(ImageStore::new(tmp.path().join("images")).expect("ImageStore::new"));
        write_sample_image(&store, "alpine", "latest");

        let dest = tmp.path().join("export");
        let tarball =
            build_docker_load_tarball(&store, "alpine", "latest", &dest).expect("build tarball");

        let file = std::fs::File::open(&tarball).expect("open tarball");
        let mut archive = tar::Archive::new(file);
        let mut config_json = None;
        for entry in archive.entries().expect("entries") {
            let mut entry = entry.expect("entry");
            if entry.path().expect("path").to_string_lossy() == "config.json" {
                let mut buf = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut buf).expect("read config.json");
                config_json = Some(buf);
            }
        }
        let config_json = config_json.expect("config.json present in tarball");
        let parsed: serde_json::Value = serde_json::from_slice(&config_json).expect("parse config");
        let diff_ids = parsed["rootfs"]["diff_ids"]
            .as_array()
            .expect("diff_ids array");
        assert_eq!(diff_ids.len(), 1);
        assert!(
            diff_ids[0]
                .as_str()
                .expect("diff id str")
                .starts_with("sha256:")
        );
    }

    #[test]
    fn errors_when_layer_dir_missing_on_disk() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(ImageStore::new(tmp.path().join("images")).expect("ImageStore::new"));
        // Manifest declares a layer that was never extracted to disk.
        let manifest = OciManifest {
            schema_version: 2,
            media_type: "application/vnd.oci.image.manifest.v1+json".to_string(),
            config: Descriptor {
                media_type: "application/vnd.oci.image.config.v1+json".to_string(),
                size: 2,
                digest: "sha256:config".to_string(),
                platform: None,
            },
            layers: vec![Descriptor {
                media_type: "application/vnd.oci.image.layer.v1.tar+gzip".to_string(),
                size: 0,
                digest: "sha256:missing".to_string(),
                platform: None,
            }],
        };
        store
            .store_manifest("alpine", "latest", &manifest)
            .expect("store manifest");
        store
            .store_config_blob("alpine", "latest", b"{}")
            .expect("store config blob");

        let dest = tmp.path().join("export");
        let result = build_docker_load_tarball(&store, "alpine", "latest", &dest);
        assert!(
            result.is_err(),
            "expected error when layer dir is absent on disk"
        );
    }
}
