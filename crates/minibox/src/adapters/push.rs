//! OCI Distribution Spec push adapter.
//!
//! Reads locally-stored image layers from [`ImageStore`] and pushes them to
//! an OCI-compliant registry using the Distribution Spec v1 upload protocol:
//!
//! 1. For each layer: HEAD to check existence, POST+PUT to upload if absent.
//! 2. Push the manifest.
//!
//! # Layer encoding
//!
//! `ImageStore` stores layers as **extracted directories**, not raw blobs.
//! This adapter re-compresses each layer directory into a gzip-compressed tar
//! before uploading. The resulting bytes may differ from the original blob
//! (different timestamps, ordering) so the digest will not match the stored
//! manifest digest. For a faithful round-trip (pull → push) a future revision
//! should cache the raw compressed blob alongside the extracted dir at pull
//! time. The current implementation is sufficient for `commit`-created images
//! where the blob is freshly produced from the container's upper dir.

use anyhow::{Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use flate2::Compression;
use flate2::write::GzEncoder;
use minibox_core::as_any;
use minibox_core::domain::{
    DynImagePusher, ImagePusher, PushProgress, PushResult, RegistryCredentials,
};
use minibox_core::image::ImageStore;
use minibox_core::image::registry::{PushAuth, RegistryClient};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tracing::info;

pub struct OciPushAdapter {
    client: RegistryClient,
    store: Arc<ImageStore>,
}

impl OciPushAdapter {
    pub fn new(client: RegistryClient, store: Arc<ImageStore>) -> Self {
        Self { client, store }
    }
}

as_any!(OciPushAdapter);

#[async_trait]
impl ImagePusher for OciPushAdapter {
    async fn push_image(
        &self,
        image_ref: &minibox_core::image::reference::ImageRef,
        credentials: &RegistryCredentials,
        progress_tx: Option<tokio::sync::mpsc::Sender<PushProgress>>,
    ) -> Result<PushResult> {
        let repo = image_ref.repository();
        let registry_host = image_ref.registry_host();
        let registry_base = format!("{}://{}", registry_scheme(registry_host), registry_host);

        let auth = if let Some(bearer) = push_auth_from_credentials(credentials) {
            bearer
        } else {
            let (username, password) = match credentials {
                RegistryCredentials::Basic { username, password } => {
                    (Some(username.as_str()), Some(password.as_str()))
                }
                _ => (None, None),
            };
            self.client
                .resolve_push_auth(&registry_base, &repo, username, password)
                .await
                .with_context(|| format!("push auth for {repo}"))?
        };

        // Load the stored manifest to get layer descriptors.
        let cache_name = image_ref.cache_name();
        let tag = &image_ref.tag;

        // We need access to the internal load_manifest and layers_dir.
        // Use the public get_image_layers to enumerate layer paths, and
        // reconstruct digests from the manifest via store_manifest path.
        let manifest = self
            .store
            .load_manifest_pub(&cache_name, tag)
            .with_context(|| format!("load manifest for {cache_name}:{tag}"))?;

        let layers_dir = self
            .store
            .layers_dir_pub(&cache_name, tag)
            .with_context(|| format!("layers dir for {cache_name}:{tag}"))?;

        let mut total_size: u64 = 0;
        let mut final_digest = String::new();

        for layer_desc in &manifest.layers {
            let digest_key = layer_desc.digest.replace(':', "_");
            let layer_dir = layers_dir.join(&digest_key);
            let layer_tar = layers_dir.join(format!("{digest_key}.tar"));

            let blob = if layer_tar.exists() {
                std::fs::read(&layer_tar)
                    .with_context(|| format!("read raw layer blob {}", layer_tar.display()))?
            } else {
                retar_layer_dir(&layer_dir)
                    .with_context(|| format!("re-tar layer {}", layer_desc.digest))?
            };

            let blob_size = blob.len() as u64;
            total_size += blob_size;

            // Compute actual digest of the re-compressed blob.
            let actual_digest = format!("sha256:{:x}", Sha256::digest(&blob));

            // Check if blob already exists in the registry.
            if self
                .client
                .blob_exists(&registry_base, &repo, &actual_digest, &auth)
                .await
            {
                info!(
                    digest = %actual_digest,
                    "push: layer already exists, skipping"
                );
            } else {
                // Notify progress start.
                if let Some(ref tx) = progress_tx {
                    let _ = tx
                        .send(PushProgress {
                            layer_digest: actual_digest.clone(),
                            bytes_uploaded: 0,
                            total_bytes: blob_size,
                        })
                        .await;
                }

                // Initiate and complete upload.
                let upload_url = self
                    .client
                    .initiate_blob_upload(&registry_base, &repo, &auth)
                    .await
                    .with_context(|| format!("initiate upload for {actual_digest}"))?;

                self.client
                    .upload_blob(&upload_url, &actual_digest, Bytes::from(blob), &auth)
                    .await
                    .with_context(|| format!("upload blob {actual_digest}"))?;

                // Notify progress complete.
                if let Some(ref tx) = progress_tx {
                    let _ = tx
                        .send(PushProgress {
                            layer_digest: actual_digest.clone(),
                            bytes_uploaded: blob_size,
                            total_bytes: blob_size,
                        })
                        .await;
                }

                info!(
                    digest = %actual_digest,
                    bytes = blob_size,
                    "push: layer uploaded"
                );
            }

            final_digest = actual_digest;
        }

        // Push the manifest.
        let manifest_digest = self
            .client
            .push_manifest(&registry_base, &repo, tag, &manifest, &auth)
            .await
            .with_context(|| format!("push manifest for {repo}:{tag}"))?;

        info!(
            repo = %repo,
            tag = %tag,
            digest = %manifest_digest,
            total_bytes = total_size,
            "push: completed"
        );

        Ok(PushResult {
            digest: if manifest_digest.is_empty() {
                final_digest
            } else {
                manifest_digest
            },
            size_bytes: total_size,
        })
    }
}

/// Re-compress an extracted layer directory into a gzip-compressed tar archive.
///
/// The resulting bytes are a valid OCI layer blob, though the digest will differ
/// from the original pull because timestamps and entry order may vary.
fn retar_layer_dir(dir: &std::path::Path) -> Result<Vec<u8>> {
    let buf = Vec::new();
    let gz = GzEncoder::new(buf, Compression::default());
    let mut builder = tar::Builder::new(gz);
    builder.follow_symlinks(false);
    builder
        .append_dir_all(".", dir)
        .with_context(|| format!("append layer dir {}", dir.display()))?;
    let gz = builder.into_inner().context("finish tar builder")?;
    let bytes = gz.finish().context("finish gzip encoder")?;
    Ok(bytes)
}

/// Map [`RegistryCredentials::Token`] directly to [`PushAuth::Bearer`].
///
/// Returns `Some(PushAuth::Bearer(token))` when the caller has already
/// obtained a bearer token (e.g. from GHCR or a CI token source) and we
/// should skip the `resolve_push_auth` round-trip entirely.  Returns `None`
/// for all other credential variants so the caller falls through to the
/// normal `resolve_push_auth` path.
fn push_auth_from_credentials(credentials: &RegistryCredentials) -> Option<PushAuth> {
    match credentials {
        RegistryCredentials::Token(token) => Some(PushAuth::Bearer(token.clone())),
        _ => None,
    }
}

fn registry_scheme(registry_host: &str) -> &'static str {
    match registry_host {
        "localhost" | "127.0.0.1" => "http",
        host if host.starts_with("localhost:") || host.starts_with("127.0.0.1:") => "http",
        _ => "https",
    }
}

/// Construct an [`OciPushAdapter`] as a [`DynImagePusher`].
pub fn oci_push_adapter(client: RegistryClient, store: Arc<ImageStore>) -> DynImagePusher {
    Arc::new(OciPushAdapter::new(client, store))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_token_maps_to_bearer_push_auth() {
        use minibox_core::image::registry::PushAuth;
        let creds = RegistryCredentials::Token("my-token".to_owned());
        let push_auth = push_auth_from_credentials(&creds);
        assert_eq!(push_auth, Some(PushAuth::Bearer("my-token".to_owned())));
    }

    #[test]
    fn credentials_basic_returns_none_from_helper() {
        let creds = RegistryCredentials::Basic {
            username: "u".to_owned(),
            password: "p".to_owned(),
        };
        let push_auth = push_auth_from_credentials(&creds);
        assert_eq!(push_auth, None);
    }

    #[test]
    fn push_adapter_constructs() {
        let store = Arc::new(
            minibox_core::image::ImageStore::new(tempfile::TempDir::new().unwrap().path()).unwrap(),
        );
        let client = RegistryClient::new().unwrap();
        let _adapter = OciPushAdapter::new(client, store);
    }

    #[test]
    fn registry_scheme_uses_http_for_localhost_targets() {
        assert_eq!(registry_scheme("localhost"), "http");
        assert_eq!(registry_scheme("localhost:5001"), "http");
        assert_eq!(registry_scheme("127.0.0.1"), "http");
        assert_eq!(registry_scheme("127.0.0.1:5001"), "http");
        assert_eq!(registry_scheme("ghcr.io"), "https");
    }
}
