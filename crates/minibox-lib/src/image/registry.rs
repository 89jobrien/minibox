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

use crate::error::{ImageError, RegistryError};
use crate::image::ImageStore;
use crate::image::layer::{HashingReader, extract_layer};
use crate::image::manifest::{
    MEDIA_TYPE_DOCKER_MANIFEST, MEDIA_TYPE_DOCKER_MANIFEST_LIST, MEDIA_TYPE_OCI_INDEX,
    MEDIA_TYPE_OCI_MANIFEST, ManifestResponse, OciManifest,
};
use anyhow::Context;
use bytes::Bytes;
use futures::StreamExt as _;
use pin_project_lite::pin_project;
use reqwest::Client;
use serde::Deserialize;
use std::io;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::io::{StreamReader, SyncIoBridge};
use tracing::{Instrument, debug, info, instrument};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const AUTH_URL: &str = "https://auth.docker.io/token";
const REGISTRY_BASE: &str = "https://registry-1.docker.io/v2";

// SECURITY: Resource limits to prevent DoS attacks
const MAX_MANIFEST_SIZE: u64 = 10 * 1024 * 1024; // 10 MB
const MAX_LAYER_SIZE: u64 = 10 * 1024 * 1024 * 1024; // 10 GB
const MAX_CONCURRENT_LAYERS: usize = 4;

// ---------------------------------------------------------------------------
// LimitedStream
// ---------------------------------------------------------------------------

pin_project! {
    /// Wraps an async byte stream and returns an error if total bytes exceed `limit`.
    ///
    /// Used to enforce [`MAX_LAYER_SIZE`] during streaming download without
    /// buffering the full blob in memory.
    struct LimitedStream<S> {
        #[pin]
        inner: S,
        remaining: u64,
    }
}

impl<S> LimitedStream<S> {
    fn new(inner: S, limit: u64) -> Self {
        Self {
            inner,
            remaining: limit,
        }
    }
}

impl<S: futures::Stream<Item = io::Result<Bytes>>> futures::Stream for LimitedStream<S> {
    type Item = io::Result<Bytes>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.project();
        match this.inner.poll_next(cx) {
            std::task::Poll::Ready(Some(Ok(chunk))) => {
                if chunk.len() as u64 > *this.remaining {
                    std::task::Poll::Ready(Some(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("layer exceeded size limit of {MAX_LAYER_SIZE} bytes"),
                    ))))
                } else {
                    *this.remaining -= chunk.len() as u64;
                    std::task::Poll::Ready(Some(Ok(chunk)))
                }
            }
            std::task::Poll::Ready(Some(Err(e))) => std::task::Poll::Ready(Some(Err(e))),
            std::task::Poll::Ready(None) => std::task::Poll::Ready(None),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

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

    /// Start a blob download and return the HTTP response for streaming.
    ///
    /// Performs status and `Content-Length` header checks before returning.
    /// The caller enforces the streaming byte limit via [`LimitedStream`].
    #[instrument(skip(self, token), fields(digest = &digest[..19]))]
    async fn pull_layer(
        &self,
        name: &str,
        digest: &str,
        token: &str,
    ) -> anyhow::Result<reqwest::Response> {
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

        // Advisory content-length check (LimitedStream enforces the hard limit).
        if let Some(content_length) = resp.headers().get("content-length")
            && let Ok(size_str) = content_length.to_str()
            && let Ok(size) = size_str.parse::<u64>()
            && size > MAX_LAYER_SIZE
        {
            return Err(RegistryError::Other(format!(
                "layer too large per Content-Length: {size} bytes (max {MAX_LAYER_SIZE})"
            ))
            .into());
        }

        Ok(resp)
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

        // 3. Download and store each layer in parallel (bounded by MAX_CONCURRENT_LAYERS).
        let total_layers = manifest.layers.len();
        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_LAYERS));
        let mut join_set: JoinSet<anyhow::Result<()>> = JoinSet::new();

        for (idx, layer_desc) in manifest.layers.iter().cloned().enumerate() {
            let client = self.clone();
            let store = store.clone();
            let token = token.clone();
            let sem = semaphore.clone();
            let name = name.to_owned();
            let tag = tag.to_owned();

            join_set.spawn(async move {
                let _permit = sem.acquire_owned().await.expect("semaphore closed");

                let layer_start = std::time::Instant::now();
                let digest = layer_desc.digest.clone();

                // Early-exit if already cached (validated path via layer_path).
                let layer_dir = store
                    .layer_path(&name, &tag, &digest)
                    .with_context(|| format!("layer path for {digest}"))?;
                if layer_dir.exists() {
                    info!(
                        "layer {}/{}: {} (cached)",
                        idx + 1,
                        total_layers,
                        &digest[..19]
                    );
                    return Ok(());
                }

                // Start the HTTP download (async).
                let response = client
                    .pull_layer(&name, &digest, &token)
                    .await
                    .with_context(|| format!("pull layer {digest}"))?;

                // Bridge async stream → sync Read inside spawn_blocking.
                let limited_stream = LimitedStream::new(
                    response.bytes_stream().map(|r| r.map_err(io::Error::other)),
                    MAX_LAYER_SIZE,
                );
                let handle = tokio::runtime::Handle::current();

                tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                    let sync_reader =
                        SyncIoBridge::new_with_handle(StreamReader::new(limited_stream), handle);
                    let mut hashing_reader = HashingReader::new(sync_reader);

                    // Compute paths.
                    let dest = store
                        .layer_path(&name, &tag, &digest)
                        .with_context(|| format!("layer path for {digest}"))?;

                    // Early return if cached (race: another task may have committed first).
                    if dest.exists() {
                        return Ok(());
                    }

                    let tmp = dest.with_extension("tmp");

                    // Clean up any stale tmp from a prior crash.
                    if tmp.exists() {
                        std::fs::remove_dir_all(&tmp)
                            .with_context(|| format!("cleaning stale tmp {tmp:?}"))?;
                    }

                    std::fs::create_dir_all(&tmp).map_err(|source| ImageError::StoreWrite {
                        path: tmp.display().to_string(),
                        source,
                    })?;

                    // Extract (bytes flow: HTTP → LimitedStream → SyncIoBridge → HashingReader → GzDecoder → tar).
                    if let Err(e) = extract_layer(&mut hashing_reader, &tmp)
                        .with_context(|| format!("extracting layer {digest} to {tmp:?}"))
                    {
                        std::fs::remove_dir_all(&tmp).ok();
                        return Err(e);
                    }

                    // Verify digest BEFORE renaming to final location.
                    let bytes = hashing_reader.bytes_read();
                    let actual = hashing_reader.finalize();
                    let expected_hex = digest
                        .strip_prefix("sha256:")
                        .ok_or_else(|| anyhow::anyhow!("unexpected digest format: {digest}"))?;

                    if actual != expected_hex {
                        std::fs::remove_dir_all(&tmp).ok(); // tmp is still at tmp path, safe to remove
                        return Err(ImageError::DigestMismatch {
                            digest: digest.clone(),
                            expected: expected_hex.to_owned(),
                            actual,
                        }
                        .into());
                    }

                    // Atomically commit: rename tmp → dest.
                    match std::fs::rename(&tmp, &dest) {
                        Ok(()) => {}
                        Err(_) if dest.exists() => {
                            // Another task won the race on a duplicate-digest layer. Clean up our tmp.
                            std::fs::remove_dir_all(&tmp).ok();
                        }
                        Err(source) => {
                            std::fs::remove_dir_all(&tmp).ok();
                            return Err(ImageError::StoreWrite {
                                path: dest.display().to_string(),
                                source,
                            }
                            .into());
                        }
                    }

                    info!(
                        "layer {}/{} ({}) done in {:.2?} — {} bytes",
                        idx + 1,
                        total_layers,
                        &digest[..19],
                        layer_start.elapsed(),
                        bytes,
                    );
                    Ok(())
                })
                .await
                .map_err(|join_err| {
                    anyhow::Error::from(RegistryError::LayerTask {
                        digest: layer_desc.digest.clone(),
                        message: join_err.to_string(),
                    })
                })??;
                Ok(())
            });
        }

        // Collect results; abort remaining tasks on first error.
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    join_set.abort_all();
                    return Err(e);
                }
                Err(join_err) => {
                    join_set.abort_all();
                    return Err(RegistryError::LayerTask {
                        digest: "(unknown)".into(),
                        message: join_err.to_string(),
                    }
                    .into());
                }
            }
        }

        // 4. Persist manifest.
        {
            let _span = tracing::debug_span!("store_manifest").entered();
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
