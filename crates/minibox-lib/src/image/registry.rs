//! Docker Hub / OCI registry client.
//!
//! Supports anonymous token authentication (sufficient for public images) and
//! pulls manifests and blobs from `registry-1.docker.io`.
//!
//! # Usage
//!
//! ```rust,no_run
//! use minibox_lib::image::{ImageStore, registry::RegistryClient};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let store = ImageStore::new("/var/lib/minibox/images")?;
//!     let client = RegistryClient::new()?;
//!     client.pull_image("library/ubuntu", "22.04", &store).await?;
//!     Ok(())
//! }
//! ```

use crate::error::RegistryError;
use crate::image::ImageStore;
use crate::image::layer::verify_digest;
use crate::image::manifest::{
    MEDIA_TYPE_DOCKER_MANIFEST, MEDIA_TYPE_DOCKER_MANIFEST_LIST, MEDIA_TYPE_OCI_INDEX,
    MEDIA_TYPE_OCI_MANIFEST, ManifestResponse, OciManifest,
};
use anyhow::Context;
use bytes::Bytes;
use futures::stream::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use tracing::{Instrument, debug, info, instrument};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const AUTH_URL: &str = "https://auth.docker.io/token";
const REGISTRY_BASE: &str = "https://registry-1.docker.io/v2";

// SECURITY: Resource limits to prevent DoS attacks
const MAX_MANIFEST_SIZE: u64 = 10 * 1024 * 1024; // 10 MB
const MAX_LAYER_SIZE: u64 = 10 * 1024 * 1024 * 1024; // 10 GB

// ---------------------------------------------------------------------------
// Auth token response
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
}

// ---------------------------------------------------------------------------
// RegistryClient
// ---------------------------------------------------------------------------

/// A Docker Hub registry client.
///
/// Internally wraps a [`reqwest::Client`] with redirect following enabled
/// (required for blob downloads which redirect to CDN).
#[derive(Debug, Clone)]
pub struct RegistryClient {
    http: Client,
}

impl RegistryClient {
    /// Create a new client with secure HTTPS-only configuration.
    ///
    /// # Security
    ///
    /// - HTTPS-only: Rejects HTTP connections to prevent MitM attacks
    /// - TLS 1.2+: Enforces minimum TLS version
    /// - Redirect limits: Max 10 redirects to prevent redirect loops
    pub fn new() -> anyhow::Result<Self> {
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .https_only(true) // SECURITY: Reject HTTP, require HTTPS
            .min_tls_version(reqwest::tls::Version::TLS_1_2) // SECURITY: Minimum TLS 1.2
            .build()
            .map_err(RegistryError::Network)?;
        Ok(Self { http })
    }

    // -----------------------------------------------------------------------
    // Authentication
    // -----------------------------------------------------------------------

    /// Obtain an anonymous pull token for `image_name` from Docker Hub.
    ///
    /// The returned token should be passed to subsequent manifest/blob calls.
    #[instrument(skip(self), fields(image = image_name))]
    pub async fn authenticate(&self, image_name: &str) -> anyhow::Result<String> {
        debug!("authenticating for image '{}'", image_name);

        let url =
            format!("{AUTH_URL}?service=registry.docker.io&scope=repository:{image_name}:pull");

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(RegistryError::Network)
            .with_context(|| format!("auth request for {image_name}"))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let msg = resp.text().await.unwrap_or_else(|_| String::new());
            return Err(RegistryError::AuthFailed {
                image: image_name.to_owned(),
                message: format!("HTTP {status}: {msg}"),
            }
            .into());
        }

        let token_resp: TokenResponse = resp
            .json::<TokenResponse>()
            .await
            .map_err(RegistryError::Network)
            .with_context(|| "parsing auth token response")?;

        info!("authenticated for '{}'", image_name);
        Ok(token_resp.token)
    }

    // -----------------------------------------------------------------------
    // Manifest
    // -----------------------------------------------------------------------

    /// Fetch the manifest for `name:tag`.
    ///
    /// Handles both single-arch manifests and manifest lists. If the registry
    /// returns a manifest list, the `linux/amd64` entry is selected and that
    /// manifest is fetched.
    #[instrument(skip(self, token), fields(image = %format!("{name}:{tag}")))]
    pub async fn get_manifest(
        &self,
        name: &str,
        tag: &str,
        token: &str,
    ) -> anyhow::Result<OciManifest> {
        let url = format!("{REGISTRY_BASE}/{name}/manifests/{tag}");
        debug!("fetching manifest {}", url);

        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .header(
                "Accept",
                format!(
                    "{MEDIA_TYPE_OCI_MANIFEST}, {MEDIA_TYPE_OCI_INDEX}, {MEDIA_TYPE_DOCKER_MANIFEST}, {MEDIA_TYPE_DOCKER_MANIFEST_LIST}",
                ),
            )
            .send()
            .await
            .map_err(RegistryError::Network)
            .with_context(|| format!("GET manifest {name}:{tag}"))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let msg = resp.text().await.unwrap_or_else(|_| String::new());
            return Err(RegistryError::ManifestFetch {
                name: name.to_owned(),
                tag: tag.to_owned(),
                message: format!("HTTP {status}: {msg}"),
            }
            .into());
        }

        // Use the Content-Type header to determine whether this is a manifest
        // list or a single manifest.
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v: &reqwest::header::HeaderValue| v.to_str().ok())
            .unwrap_or("")
            .to_owned();

        // SECURITY: Check Content-Length header before reading
        if let Some(content_length) = resp.headers().get("content-length")
            && let Ok(size_str) = content_length.to_str()
            && let Ok(size) = size_str.parse::<u64>()
            && size > MAX_MANIFEST_SIZE
        {
            return Err(RegistryError::Other(format!(
                "manifest too large: {size} bytes (max {MAX_MANIFEST_SIZE})"
            ))
            .into());
        }

        // SECURITY: Use streaming reader with size limit
        let mut body_stream = resp.bytes_stream();
        let mut body = Vec::new();

        while let Some(chunk_result) = body_stream.next().await {
            let chunk = chunk_result.map_err(RegistryError::Network)?;
            body.extend_from_slice(&chunk);

            if body.len() as u64 > MAX_MANIFEST_SIZE {
                return Err(RegistryError::Other(format!(
                    "manifest exceeded size limit: {MAX_MANIFEST_SIZE} bytes"
                ))
                .into());
            }
        }

        let body = Bytes::from(body);
        debug!(
            "manifest response content-type='{}' size={}",
            content_type,
            body.len()
        );

        match ManifestResponse::parse(&body, &content_type)? {
            ManifestResponse::Single(m) => Ok(m),
            ManifestResponse::List(list) => {
                // Find linux/amd64 and fetch that manifest.
                let amd64 = list
                    .find_linux_amd64()
                    .ok_or(RegistryError::NoAmd64Manifest)?;
                info!("manifest list resolved to amd64 digest={}", amd64.digest);
                let digest = amd64.digest.clone();
                // Recurse with the digest as the "tag".
                Box::pin(self.get_manifest(name, &digest, token)).await
            }
        }
    }

    // -----------------------------------------------------------------------
    // Blob / layer pull
    // -----------------------------------------------------------------------

    /// Download a single blob by `digest` and return its raw bytes.
    ///
    /// Docker Hub redirects blob requests to a CDN; the client follows these
    /// redirects automatically.
    #[instrument(skip(self, token), fields(digest = &digest[..19]))]
    pub async fn pull_layer(&self, name: &str, digest: &str, token: &str) -> anyhow::Result<Bytes> {
        let url = format!("{REGISTRY_BASE}/{name}/blobs/{digest}");
        debug!("pulling blob {} from {}", digest, url);

        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(RegistryError::Network)
            .with_context(|| format!("GET blob {digest}"))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let msg = resp.text().await.unwrap_or_else(|_| String::new());
            return Err(RegistryError::BlobFetch {
                digest: digest.to_owned(),
                message: format!("HTTP {status}: {msg}"),
            }
            .into());
        }

        // SECURITY: Check Content-Length header before downloading
        if let Some(content_length) = resp.headers().get("content-length")
            && let Ok(size_str) = content_length.to_str()
            && let Ok(size) = size_str.parse::<u64>()
        {
            if size > MAX_LAYER_SIZE {
                return Err(RegistryError::Other(format!(
                    "layer too large: {size} bytes (max {MAX_LAYER_SIZE})"
                ))
                .into());
            }
            debug!("layer size: {} bytes", size);
        }

        // SECURITY: Use streaming reader with size limit
        let mut body_stream = resp.bytes_stream();
        let mut data = Vec::new();

        while let Some(chunk_result) = body_stream.next().await {
            let chunk = chunk_result.map_err(RegistryError::Network)?;
            data.extend_from_slice(&chunk);

            if data.len() as u64 > MAX_LAYER_SIZE {
                return Err(RegistryError::Other(format!(
                    "layer exceeded size limit during download: {MAX_LAYER_SIZE} bytes"
                ))
                .into());
            }
        }

        let data = Bytes::from(data);
        info!("pulled blob {} ({} bytes)", digest, data.len());
        Ok(data)
    }

    // -----------------------------------------------------------------------
    // High-level pull
    // -----------------------------------------------------------------------

    /// Pull a complete image and store it locally.
    ///
    /// Steps:
    /// 1. Authenticate.
    /// 2. Fetch manifest (resolving manifest lists automatically).
    /// 3. For each layer: download blob, verify digest, extract to store.
    /// 4. Persist manifest.
    #[instrument(skip(self, store), fields(image = %format!("{name}:{tag}")))]
    pub async fn pull_image(
        &self,
        name: &str,
        tag: &str,
        store: &ImageStore,
    ) -> anyhow::Result<()> {
        let pull_start = std::time::Instant::now();
        info!("pulling image {}:{}", name, tag);

        // 1. Authenticate.
        let t = std::time::Instant::now();
        let token = self
            .authenticate(name)
            .instrument(tracing::info_span!("auth"))
            .await
            .with_context(|| format!("authenticate for {name}"))?;
        info!("auth completed in {:.2?}", t.elapsed());

        // 2. Fetch manifest.
        let t = std::time::Instant::now();
        let manifest = self
            .get_manifest(name, tag, &token)
            .instrument(tracing::info_span!("manifest"))
            .await
            .with_context(|| format!("get manifest for {name}:{tag}"))?;
        info!(
            "manifest fetched in {:.2?} ({} layers)",
            t.elapsed(),
            manifest.layers.len()
        );

        // 3. Download and store each layer.
        for (idx, layer_desc) in manifest.layers.iter().enumerate() {
            // Build the expected layer directory path to check for existence.
            let digest_key = layer_desc.digest.replace(':', "_");
            let layer_dir = store
                .base_dir
                .join(name.replace('/', "_"))
                .join(tag)
                .join("layers")
                .join(&digest_key);

            if layer_dir.exists() {
                info!(
                    "layer {}/{}: {} (cached)",
                    idx + 1,
                    manifest.layers.len(),
                    &layer_desc.digest[..19]
                );
                continue;
            }

            let layer_span = tracing::info_span!(
                "layer",
                n = idx + 1,
                total = manifest.layers.len(),
                digest = &layer_desc.digest[..19],
            );

            let layer_start = std::time::Instant::now();
            let t = std::time::Instant::now();
            let data = self
                .pull_layer(name, &layer_desc.digest, &token)
                .instrument(layer_span.clone())
                .await
                .with_context(|| format!("pull layer {}", layer_desc.digest))?;
            let download_ms = t.elapsed();

            let _guard = layer_span.enter();

            let t = std::time::Instant::now();
            {
                let _span = tracing::info_span!("verify_digest").entered();
                verify_digest(&data, &layer_desc.digest)
                    .with_context(|| format!("digest verification for {}", layer_desc.digest))?;
            }
            let verify_ms = t.elapsed();

            let t = std::time::Instant::now();
            {
                let _span = tracing::info_span!("extract", bytes = data.len()).entered();
                store
                    .store_layer(name, tag, &layer_desc.digest, std::io::Cursor::new(data))
                    .with_context(|| format!("store layer {}", layer_desc.digest))?;
            }
            let extract_ms = t.elapsed();

            drop(_guard);

            info!(
                "layer {}/{} ({}) done in {:.2?} — download {:.2?} verify {:.2?} extract {:.2?}",
                idx + 1,
                manifest.layers.len(),
                &layer_desc.digest[..19],
                layer_start.elapsed(),
                download_ms,
                verify_ms,
                extract_ms,
            );
        }

        // 4. Persist manifest.
        {
            let _span = tracing::info_span!("store_manifest").entered();
            store
                .store_manifest(name, tag, &manifest)
                .with_context(|| format!("store manifest for {name}:{tag}"))?;
        }

        info!(
            "image {}:{} pulled in {:.2?}",
            name,
            tag,
            pull_start.elapsed()
        );
        Ok(())
    }
}

impl Default for RegistryClient {
    fn default() -> Self {
        Self::new().expect("failed to build RegistryClient")
    }
}
