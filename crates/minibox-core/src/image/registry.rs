//! Docker Hub / OCI registry client.
//!
//! Supports anonymous token authentication (sufficient for public images) and
//! pulls manifests and blobs from `registry-1.docker.io`.
//!
//! # Usage
//!
//! ```rust,no_run
//! use minibox_core::image::{ImageStore, registry::RegistryClient};
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
use crate::image::layer::{HashingReader, extract_layer};
use crate::image::manifest::{
    MEDIA_TYPE_DOCKER_MANIFEST, MEDIA_TYPE_DOCKER_MANIFEST_LIST, MEDIA_TYPE_OCI_INDEX,
    MEDIA_TYPE_OCI_MANIFEST, ManifestResponse, OciManifest,
};
use anyhow::Context;
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use pin_project_lite::pin_project;
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::io::{StreamReader, SyncIoBridge};
use tracing::{Instrument, debug, info, instrument, warn};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const AUTH_URL: &str = "https://auth.docker.io/token";
const REGISTRY_BASE: &str = "https://registry-1.docker.io/v2";

// SECURITY: Resource limits to prevent DoS attacks.
// MAX_MANIFEST_SIZE: manifests are small JSON blobs; 10 MB is a generous ceiling.
// MAX_LAYER_SIZE: individual compressed layer blobs; 10 GB allows large images while
// bounding memory consumption during streaming download.
const MAX_MANIFEST_SIZE: u64 = 10 * 1024 * 1024; // 10 MiB
const MAX_LAYER_SIZE: u64 = 10 * 1024 * 1024 * 1024; // 10 GiB

/// Maximum number of layer blobs downloaded concurrently.
const MAX_CONCURRENT_LAYERS: usize = 4;

// ---------------------------------------------------------------------------
// LimitedStream
// ---------------------------------------------------------------------------

pin_project! {
    /// Async stream wrapper that enforces a hard byte ceiling on the wrapped stream.
    ///
    /// # Contract
    ///
    /// - **What is counted**: raw bytes on the wire (compressed), not decompressed tar
    ///   contents. For gzip-compressed OCI layers this means the HTTP response body bytes,
    ///   *before* decompression by `GzDecoder`. The limit applies to compressed layer size.
    ///
    /// - **Content-Length**: `LimitedStream` does not inspect `Content-Length`; that check
    ///   is the caller's responsibility (see [`RegistryClient::pull_layer_response`]).
    ///   Content-Length mismatches should be rejected *before* wrapping in `LimitedStream`.
    ///   This streaming limit acts as a second, independent cap.
    ///
    /// - **Boundary**: exactly `limit` bytes is **allowed**; `limit + 1` bytes triggers the
    ///   error. Formally: `consumed > limit` → error.
    ///
    /// - **Error kind**: when the limit is exceeded the poll returns
    ///   `Poll::Ready(Some(Err(io::Error::new(io::ErrorKind::InvalidData, …))))`.
    ///   The caller should drop the stream after receiving this error.
    ///
    /// - **Error precedence**: an `Err` yielded by the inner stream is forwarded as-is
    ///   (via `e.into()`) without checking whether the byte count was also exceeded.
    ///
    /// - **Premature EOF**: if the underlying stream ends before all expected bytes are
    ///   consumed, `LimitedStream` returns `Poll::Ready(None)`. Upper layers
    ///   (`StreamReader` / `SyncIoBridge`) surface that as an unexpected-EOF `io::Error`
    ///   during decompression or digest verification.
    pub struct LimitedStream<S> {
        #[pin]
        inner: S,
        limit: u64,
        consumed: u64,
    }
}

impl<S> LimitedStream<S> {
    /// Wrap `inner` with a `limit`-byte ceiling.
    pub fn new(inner: S, limit: u64) -> Self {
        Self {
            inner,
            limit,
            consumed: 0,
        }
    }

    /// Bytes consumed so far.
    pub fn consumed(&self) -> u64 {
        self.consumed
    }
}

impl<S, E> Stream for LimitedStream<S>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: Into<io::Error>,
{
    type Item = Result<Bytes, io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        match this.inner.poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e.into()))),
            Poll::Ready(Some(Ok(chunk))) => {
                *this.consumed += chunk.len() as u64;
                if *this.consumed > *this.limit {
                    Poll::Ready(Some(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "layer stream exceeded size limit: {} bytes (max {})",
                            this.consumed, this.limit
                        ),
                    ))))
                } else {
                    Poll::Ready(Some(Ok(chunk)))
                }
            }
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

/// Registry authentication for push operations.
///
/// `Debug` is manually implemented to redact secrets.
#[derive(Clone, PartialEq, Eq)]
pub enum PushAuth {
    None,
    Basic { username: String, password: String },
    Bearer(String),
}

impl std::fmt::Debug for PushAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "PushAuth::None"),
            Self::Basic { username, .. } => {
                write!(
                    f,
                    "PushAuth::Basic {{ username: {username:?}, password: [REDACTED] }}"
                )
            }
            Self::Bearer(_) => write!(f, "PushAuth::Bearer([REDACTED])"),
        }
    }
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
    insecure_http: Client,
    auth_url: String,
    registry_base: String,
    /// Target platform for multi-arch manifest selection.
    pub platform: crate::image::manifest::TargetPlatform,
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
        let insecure_http = Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(RegistryError::Network)?;
        Ok(Self {
            http,
            insecure_http,
            auth_url: AUTH_URL.to_owned(),
            registry_base: REGISTRY_BASE.to_owned(),
            platform: crate::image::manifest::TargetPlatform::default(),
        })
    }

    /// Create a new client targeting a specific platform.
    ///
    /// Use this when pulling images for a non-host architecture (e.g.
    /// cross-platform builds).
    pub fn with_platform(platform: crate::image::manifest::TargetPlatform) -> anyhow::Result<Self> {
        let mut client = Self::new()?;
        client.platform = platform;
        Ok(client)
    }

    /// Create a client with custom base URLs for testing against a mock server.
    ///
    /// Does not enforce HTTPS — the test server runs on plain HTTP.
    #[cfg(test)]
    pub(crate) fn for_test(auth_url: &str, registry_base: &str) -> anyhow::Result<Self> {
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(RegistryError::Network)?;
        Ok(Self {
            insecure_http: http.clone(),
            http,
            auth_url: auth_url.to_owned(),
            registry_base: registry_base.to_owned(),
            platform: crate::image::manifest::TargetPlatform::default(),
        })
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

        let url = format!(
            "{}?service=registry.docker.io&scope=repository:{image_name}:pull",
            self.auth_url
        );

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
    /// manifest is fetched. Manifest list nesting is capped at 2 levels to
    /// prevent unbounded recursion from malformed or adversarial registries.
    #[instrument(skip(self, token), fields(image = %format!("{name}:{tag}")))]
    pub async fn get_manifest(
        &self,
        name: &str,
        tag: &str,
        token: &str,
    ) -> anyhow::Result<OciManifest> {
        Box::pin(self.get_manifest_inner(name, tag, token, 0)).await
    }

    async fn get_manifest_inner(
        &self,
        name: &str,
        tag: &str,
        token: &str,
        depth: u8,
    ) -> anyhow::Result<OciManifest> {
        if depth > 2 {
            return Err(RegistryError::ManifestNestingTooDeep.into());
        }

        let url = format!("{}/{name}/manifests/{tag}", self.registry_base);
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
                let amd64 = list.find_platform(&self.platform).ok_or(
                    RegistryError::NoPlatformManifest {
                        platform: self.platform.to_string(),
                    },
                )?;
                info!(
                    platform = %self.platform,
                    digest = %amd64.digest,
                    "manifest list resolved to platform-specific digest"
                );
                let digest = amd64.digest.clone();
                // Recurse with the digest as the "tag", incrementing depth to
                // guard against malformed or adversarial chained manifest lists.
                Box::pin(self.get_manifest_inner(name, &digest, token, depth + 1)).await
            }
        }
    }

    // -----------------------------------------------------------------------
    // Blob / layer pull
    // -----------------------------------------------------------------------

    /// Download a single blob by `digest` and return the HTTP response for streaming.
    ///
    /// Docker Hub redirects blob requests to a CDN; the client follows these
    /// redirects automatically. The caller streams the response body and enforces
    /// size limits via [`LimitedStream`].
    #[instrument(skip(self, token), fields(digest = %digest.get(..19).unwrap_or(digest)))]
    async fn pull_layer_response(
        &self,
        name: &str,
        digest: &str,
        token: &str,
    ) -> anyhow::Result<reqwest::Response> {
        let url = format!("{}/{name}/blobs/{digest}", self.registry_base);
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

        // SECURITY: Check Content-Length header before streaming
        if let Some(content_length) = resp.headers().get("content-length")
            && let Ok(size_str) = content_length.to_str()
            && let Ok(size) = size_str.parse::<u64>()
            && size > MAX_LAYER_SIZE
        {
            return Err(RegistryError::Other(format!(
                "layer too large: {size} bytes (max {MAX_LAYER_SIZE})"
            ))
            .into());
        }

        Ok(resp)
    }

    /// Download a single blob and return its raw bytes.
    ///
    /// Used by callers that need the full blob in memory (e.g. push adapters).
    /// For bulk image pulls, prefer [`pull_image`] which uses the parallel streaming path.
    pub async fn pull_layer(&self, name: &str, digest: &str, token: &str) -> anyhow::Result<Bytes> {
        let resp = self.pull_layer_response(name, digest, token).await?;
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
    /// 3. Download all layers in parallel (up to [`MAX_CONCURRENT_LAYERS`] at once):
    ///    - Each layer streams through [`LimitedStream`] → [`SyncIoBridge`] →
    ///      [`HashingReader`] → `GzDecoder` → `tar::Archive`.
    ///    - Digest is verified before the tmp dir is atomically renamed to its final
    ///      location.
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

        // 3. Download all layers in parallel, up to MAX_CONCURRENT_LAYERS at once.
        //
        // Each task returns (digest, result) so that JoinErrors at the drain
        // site can include the layer digest rather than the unhelpful "(unknown)"
        // placeholder. The digest is captured from `layer_desc` at spawn time
        // and echoed back regardless of success or failure.
        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_LAYERS));
        let mut join_set: JoinSet<(String, anyhow::Result<()>)> = JoinSet::new();
        let total_layers = manifest.layers.len();

        for (idx, layer_desc) in manifest.layers.iter().cloned().enumerate() {
            let sem = semaphore.clone();
            let client = self.clone();
            let store = store.clone();
            let name = name.to_owned();
            let tag = tag.to_owned();
            let token = token.clone();
            // Capture digest at spawn time so JoinErrors at the drain site
            // can identify which layer's task panicked or was cancelled.
            let task_digest = layer_desc.digest.clone();

            join_set.spawn(async move {
                // Run all layer logic in an inner async block so `?` can be
                // used throughout; the digest is always paired with the result
                // so JoinErrors at the drain site carry an actionable digest.
                let result: anyhow::Result<()> = async {
                    let _permit = sem.acquire_owned().await.expect("semaphore closed");

                    let digest = &layer_desc.digest;
                    let digest_key = digest.replace(':', "_");
                    let layer_dir = store
                        .base_dir
                        .join(name.replace('/', "_"))
                        .join(&tag)
                        .join("layers")
                        .join(&digest_key);

                    let digest_short = digest.get(..19).unwrap_or(digest);

                    // Early exit if the layer is already cached.
                    if layer_dir.exists() {
                        info!(
                            "layer {}/{}: {} (cached)",
                            idx + 1,
                            total_layers,
                            digest_short
                        );
                        return Ok(());
                    }

                    let layer_start = std::time::Instant::now();

                    // HTTP GET the blob.
                    let response = client
                        .pull_layer_response(&name, digest, &token)
                        .await
                        .with_context(|| format!("pull_layer_response for {digest}"))?;

                    // Wrap the byte stream with a size cap.
                    let limited = LimitedStream::new(
                        response.bytes_stream().map(|r| r.map_err(io::Error::other)),
                        MAX_LAYER_SIZE,
                    );

                    let handle = tokio::runtime::Handle::current();
                    let digest_owned = digest.to_owned();
                    // Clone for the JoinError mapping below; the original is
                    // moved into the spawn_blocking closure.
                    let digest_for_err = digest_owned.clone();

                    // Bridge async → sync for tar/gz extraction.
                    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                        let sync_reader =
                            SyncIoBridge::new_with_handle(StreamReader::new(limited), handle);

                        // Byte flow:
                        // HTTP → LimitedStream → StreamReader → SyncIoBridge
                        //      → HashingReader → GzDecoder → tar::Archive
                        let mut hashing_reader = HashingReader::new(sync_reader);

                        // Prepare tmp dir adjacent to the final dest.
                        let tmp_dir = {
                            let mut p = layer_dir.clone();
                            let stem = p
                                .file_name()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "layer".to_owned());
                            p.set_file_name(format!("{stem}.tmp"));
                            p
                        };

                        if tmp_dir.exists() {
                            std::fs::remove_dir_all(&tmp_dir)
                                .with_context(|| format!("remove stale tmp {tmp_dir:?}"))?;
                        }
                        std::fs::create_dir_all(&tmp_dir)
                            .with_context(|| format!("create tmp dir {tmp_dir:?}"))?;

                        // Extract into tmp dir.
                        let extract_result = extract_layer(&mut hashing_reader, &tmp_dir);

                        // Drain any remaining bytes so HashingReader covers the full
                        // compressed stream — needed for digest verification even when
                        // extraction fails partway through (e.g. bad gzip header).
                        if extract_result.is_err() {
                            let _ = std::io::copy(&mut hashing_reader, &mut std::io::sink());
                        }

                        // Verify digest before committing or surfacing extract error.
                        // A digest mismatch is the root cause — prefer it over gz errors.
                        let actual_hex = hashing_reader.finalize();
                        let expected_hex =
                            digest_owned.strip_prefix("sha256:").ok_or_else(|| {
                                anyhow::anyhow!("digest missing sha256: prefix: {digest_owned}")
                            })?;

                        let digest_ok = actual_hex == expected_hex;

                        if !digest_ok {
                            if let Err(ce) = std::fs::remove_dir_all(&tmp_dir) {
                                warn!(
                                    digest = %digest_owned,
                                    error = %ce,
                                    "layer: failed to clean up tmp dir after digest mismatch"
                                );
                            }
                            return Err(crate::error::ImageError::DigestMismatch {
                                digest: digest_owned.clone(),
                                expected: expected_hex.to_owned(),
                                actual: actual_hex,
                            }
                            .into());
                        }

                        // Digest matched — surface any extraction error now.
                        if let Err(e) = extract_result {
                            if let Err(ce) = std::fs::remove_dir_all(&tmp_dir) {
                                warn!(
                                    digest = %digest_owned,
                                    error = %ce,
                                    "layer: failed to clean up tmp dir after extract error"
                                );
                            }
                            return Err(e).with_context(|| format!("extract layer {digest_owned}"));
                        }

                        // Atomic rename: tmp → final dest.
                        if let Err(e) = std::fs::rename(&tmp_dir, &layer_dir) {
                            // Another concurrent task may have won the race.
                            if layer_dir.exists() {
                                let _ = std::fs::remove_dir_all(&tmp_dir);
                                return Ok(());
                            }
                            let _ = std::fs::remove_dir_all(&tmp_dir);
                            return Err(e)
                                .with_context(|| format!("rename {tmp_dir:?} → {layer_dir:?}"));
                        }

                        Ok(())
                    })
                    .await
                    .map_err(|e| RegistryError::LayerTask {
                        digest: digest_for_err,
                        source: e,
                    })??;

                    info!(
                        "layer {}/{} ({}) done in {:.2?}",
                        idx + 1,
                        total_layers,
                        digest_short,
                        layer_start.elapsed(),
                    );

                    Ok(())
                }
                .await;
                (task_digest, result)
            });
        }

        // Drain: all tasks must succeed.
        while let Some(join_result) = join_set.join_next().await {
            let (digest, result) = join_result.map_err(|e| RegistryError::LayerTask {
                digest: "(outer task panicked or was cancelled)".to_owned(),
                source: e,
            })?;
            result.with_context(|| format!("layer digest {digest}"))?;
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

    // -----------------------------------------------------------------------
    // Push support
    // -----------------------------------------------------------------------

    /// Resolve the authentication mode for pushing to `registry_base`.
    ///
    /// Docker Hub requires a bearer token from `auth.docker.io`, while local
    /// anonymous registries can be used without auth. For non-Docker registries
    /// with explicit credentials, reuse HTTP Basic Auth directly.
    pub async fn resolve_push_auth(
        &self,
        registry_base: &str,
        repo: &str,
        username: Option<&str>,
        password: Option<&str>,
    ) -> anyhow::Result<PushAuth> {
        if registry_base != self.registry_base {
            return Ok(match (username, password) {
                (Some(username), Some(password)) => PushAuth::Basic {
                    username: username.to_owned(),
                    password: password.to_owned(),
                },
                _ => PushAuth::None,
            });
        }

        let url = format!(
            "{}?service=registry.docker.io&scope=repository:{repo}:push,pull",
            self.auth_url
        );

        let req = self.http.get(&url);
        let req = match (username, password) {
            (Some(u), Some(p)) => req.basic_auth(u, Some(p)),
            _ => req,
        };

        let resp = req
            .send()
            .await
            .map_err(RegistryError::Network)
            .with_context(|| format!("push auth request for {repo}"))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let msg = resp.text().await.unwrap_or_default();
            return Err(RegistryError::AuthFailed {
                image: repo.to_owned(),
                message: format!("HTTP {status}: {msg}"),
            }
            .into());
        }

        let token_resp: TokenResponse = resp
            .json::<TokenResponse>()
            .await
            .map_err(RegistryError::Network)
            .with_context(|| "parsing push auth token response")?;

        Ok(PushAuth::Bearer(token_resp.token))
    }

    fn with_push_auth(&self, req: RequestBuilder, auth: &PushAuth) -> RequestBuilder {
        match auth {
            PushAuth::None => req,
            PushAuth::Basic { username, password } => req.basic_auth(username, Some(password)),
            PushAuth::Bearer(token) => req.bearer_auth(token),
        }
    }

    fn push_request(&self, method: reqwest::Method, url: &str, auth: &PushAuth) -> RequestBuilder {
        let client = if url.starts_with("http://") {
            &self.insecure_http
        } else {
            &self.http
        };
        self.with_push_auth(client.request(method, url), auth)
    }

    fn upload_url_with_digest(&self, upload_url: &str, digest: &str) -> String {
        let separator = if upload_url.contains('?') { '&' } else { '?' };
        format!("{upload_url}{separator}digest={digest}")
    }

    /// Check whether a blob already exists in the registry (HEAD request).
    pub async fn blob_exists(
        &self,
        registry_base: &str,
        repo: &str,
        digest: &str,
        auth: &PushAuth,
    ) -> bool {
        let url = format!("{registry_base}/v2/{repo}/blobs/{digest}");
        self.push_request(reqwest::Method::HEAD, &url, auth)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Start a new blob upload session and return the `Location` upload URL.
    pub async fn initiate_blob_upload(
        &self,
        registry_base: &str,
        repo: &str,
        auth: &PushAuth,
    ) -> anyhow::Result<String> {
        let url = format!("{registry_base}/v2/{repo}/blobs/uploads/");
        let resp = self
            .push_request(reqwest::Method::POST, &url, auth)
            .send()
            .await
            .context("initiate blob upload")?;
        let location = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .context("missing Location header from blob upload initiation")?
            .to_string();
        Ok(location)
    }

    /// Upload a blob via PUT, appending the `digest` query parameter.
    pub async fn upload_blob(
        &self,
        upload_url: &str,
        digest: &str,
        data: Bytes,
        auth: &PushAuth,
    ) -> anyhow::Result<()> {
        let url = self.upload_url_with_digest(upload_url, digest);
        let resp = self
            .push_request(reqwest::Method::PUT, &url, auth)
            .header("Content-Type", "application/octet-stream")
            .body(data)
            .send()
            .await
            .context("upload blob")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("blob upload failed {status}: {body}");
        }
        Ok(())
    }

    /// Push a manifest to `registry_base/v2/{repo}/manifests/{reference}`.
    ///
    /// Returns the `Docker-Content-Digest` header value (the canonical digest
    /// assigned by the registry).
    pub async fn push_manifest(
        &self,
        registry_base: &str,
        repo: &str,
        reference: &str,
        manifest: &crate::image::manifest::OciManifest,
        auth: &PushAuth,
    ) -> anyhow::Result<String> {
        let url = format!("{registry_base}/v2/{repo}/manifests/{reference}");
        let body = serde_json::to_vec(manifest).context("serializing manifest")?;
        let resp = self
            .push_request(reqwest::Method::PUT, &url, auth)
            .header("Content-Type", "application/vnd.oci.image.manifest.v1+json")
            .body(body)
            .send()
            .await
            .context("push manifest")?;
        let digest = resp
            .headers()
            .get("docker-content-digest")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("manifest push failed {status}: {body}");
        }
        Ok(digest)
    }
}

impl Default for RegistryClient {
    /// Create a default [`RegistryClient`].
    ///
    /// Panics if the underlying TLS stack cannot be initialised (extremely
    /// unlikely in practice). Prefer [`RegistryClient::new`] where you need
    /// proper error propagation.
    fn default() -> Self {
        Self::new().expect("failed to build RegistryClient")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the size constants match the documented security limits.
    #[test]
    fn test_constants_manifest_size() {
        assert_eq!(
            MAX_MANIFEST_SIZE,
            10 * 1024 * 1024,
            "MAX_MANIFEST_SIZE should be 10 MiB"
        );
    }

    #[test]
    fn test_constants_layer_size() {
        assert_eq!(
            MAX_LAYER_SIZE,
            10 * 1024 * 1024 * 1024,
            "MAX_LAYER_SIZE should be 10 GiB"
        );
    }

    /// `RegistryClient::new()` must succeed without network access — it only
    /// builds an in-process TLS-capable HTTP client.
    #[test]
    fn test_registry_client_new() {
        let client = RegistryClient::new();
        assert!(client.is_ok(), "RegistryClient::new() should succeed");
    }

    /// `Default` must not panic.
    #[test]
    fn test_registry_client_default() {
        let _client = RegistryClient::default();
    }

    /// `Clone` must produce an independent, usable client.
    #[test]
    fn test_registry_client_clone() {
        let original = RegistryClient::new().expect("RegistryClient::new() failed");
        let _cloned = original.clone();
    }

    /// `Debug` must be implemented and produce non-empty output.
    #[test]
    fn test_registry_client_debug() {
        let client = RegistryClient::new().expect("RegistryClient::new() failed");
        let debug_str = format!("{client:?}");
        assert!(!debug_str.is_empty(), "Debug output should not be empty");
    }

    #[test]
    fn upload_url_with_digest_appends_with_question_mark_when_no_query_exists() {
        let client = RegistryClient::default();
        let url = client.upload_url_with_digest(
            "http://127.0.0.1:5001/v2/repo/blobs/uploads/upload-id",
            "sha256:abc",
        );
        assert_eq!(
            url,
            "http://127.0.0.1:5001/v2/repo/blobs/uploads/upload-id?digest=sha256:abc"
        );
    }

    #[test]
    fn upload_url_with_digest_appends_with_ampersand_when_query_exists() {
        let client = RegistryClient::default();
        let url = client.upload_url_with_digest(
            "http://127.0.0.1:5001/v2/repo/blobs/uploads/upload-id?_state=token",
            "sha256:abc",
        );
        assert_eq!(
            url,
            "http://127.0.0.1:5001/v2/repo/blobs/uploads/upload-id?_state=token&digest=sha256:abc"
        );
    }

    // -------------------------------------------------------------------------
    // HTTP behaviour tests (wiremock)
    // -------------------------------------------------------------------------

    mod http {
        use super::super::*;
        use crate::image::ImageStore;
        use flate2::{Compression, write::GzEncoder};
        use serde_json::json;
        use sha2::{Digest as ShaDigest, Sha256};
        use std::io::Write;
        use tempfile::TempDir;
        use wiremock::matchers::{method, path, path_regex, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        /// Build a RegistryClient pointed at the given mock server (plain HTTP,
        /// no TLS requirement).
        fn test_client(server: &MockServer) -> RegistryClient {
            let mut client = RegistryClient::for_test(
                &format!("{}/token", server.uri()),
                &format!("{}/v2", server.uri()),
            )
            .unwrap();
            // Pin to linux/amd64 so tests are deterministic across host platforms.
            client.platform = crate::image::manifest::TargetPlatform::linux_amd64();
            client
        }

        /// Build a minimal valid gzip-compressed tar layer and return
        /// (bytes, sha256-digest).
        fn make_test_layer() -> (Vec<u8>, String) {
            let data = b"minibox test layer content";
            let mut header = tar::Header::new_gnu();
            header.set_path("hello.txt").unwrap();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();

            let mut tar_buf = Vec::new();
            {
                let mut builder = tar::Builder::new(&mut tar_buf);
                builder.append(&header, data.as_ref()).unwrap();
                builder.finish().unwrap();
            }

            let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
            gz.write_all(&tar_buf).unwrap();
            let bytes = gz.finish().unwrap();

            let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
            (bytes, digest)
        }

        // ------------------------------------------------------------------
        // authenticate
        // ------------------------------------------------------------------

        #[tokio::test]
        async fn authenticate_returns_token_on_success() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/token"))
                .and(query_param("service", "registry.docker.io"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"token": "tok_abc123"})),
                )
                .expect(1)
                .mount(&server)
                .await;

            let token = test_client(&server)
                .authenticate("library/alpine")
                .await
                .unwrap();
            assert_eq!(token, "tok_abc123");
        }

        #[tokio::test]
        async fn authenticate_errors_on_http_401() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/token"))
                .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
                .mount(&server)
                .await;

            let err = test_client(&server)
                .authenticate("library/alpine")
                .await
                .unwrap_err();
            let msg = err.to_string();
            assert!(msg.contains("HTTP 401"), "unexpected error: {msg}");
        }

        #[tokio::test]
        async fn authenticate_errors_on_bad_json() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/token"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/json")
                        .set_body_string("not json"),
                )
                .mount(&server)
                .await;

            let result = test_client(&server).authenticate("library/alpine").await;
            assert!(result.is_err(), "should fail on invalid JSON token body");
        }

        #[tokio::test]
        async fn resolve_push_auth_uses_bearer_for_docker_hub() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/token"))
                .and(query_param("service", "registry.docker.io"))
                .and(query_param("scope", "repository:library/alpine:push,pull"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"token": "push_tok"})),
                )
                .expect(1)
                .mount(&server)
                .await;

            let auth = test_client(&server)
                .resolve_push_auth(
                    &format!("{}/v2", server.uri()),
                    "library/alpine",
                    None,
                    None,
                )
                .await
                .unwrap();

            assert_eq!(auth, PushAuth::Bearer("push_tok".to_owned()));
        }

        #[tokio::test]
        async fn resolve_push_auth_uses_none_for_non_docker_registry_without_credentials() {
            let server = MockServer::start().await;

            let auth = test_client(&server)
                .resolve_push_auth("http://127.0.0.1:5001", "dogfood/mac-build", None, None)
                .await
                .unwrap();

            assert_eq!(auth, PushAuth::None);
        }

        #[tokio::test]
        async fn resolve_push_auth_uses_basic_for_non_docker_registry_with_credentials() {
            let server = MockServer::start().await;

            let auth = test_client(&server)
                .resolve_push_auth("https://ghcr.io", "org/image", Some("joe"), Some("secret"))
                .await
                .unwrap();

            assert_eq!(
                auth,
                PushAuth::Basic {
                    username: "joe".to_owned(),
                    password: "secret".to_owned(),
                }
            );
        }

        // ------------------------------------------------------------------
        // get_manifest
        // ------------------------------------------------------------------

        #[tokio::test]
        async fn get_manifest_returns_single_oci_manifest() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v2/library/alpine/manifests/latest"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/vnd.oci.image.manifest.v1+json")
                        .set_body_json(json!({
                            "schemaVersion": 2,
                            "mediaType": "application/vnd.oci.image.manifest.v1+json",
                            "config": {
                                "mediaType": "application/vnd.oci.image.config.v1+json",
                                "size": 100,
                                "digest": "sha256:cfg"
                            },
                            "layers": [
                                {
                                    "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                                    "size": 5678,
                                    "digest": "sha256:layerabc"
                                }
                            ]
                        })),
                )
                .expect(1)
                .mount(&server)
                .await;

            let manifest = test_client(&server)
                .get_manifest("library/alpine", "latest", "tok")
                .await
                .unwrap();

            assert_eq!(manifest.schema_version, 2);
            assert_eq!(manifest.layers.len(), 1);
            assert_eq!(manifest.layers[0].digest, "sha256:layerabc");
        }

        #[tokio::test]
        async fn get_manifest_resolves_manifest_list_to_amd64() {
            let server = MockServer::start().await;
            let amd64_digest = "sha256:amd64manifestdigest";

            // Use set_body_raw so we control the Content-Type precisely.
            // wiremock's set_body_json/set_body_string store the mime in a separate
            // `self.mime` field that overwrites any insert_header content-type during
            // generate_response(); set_body_raw avoids that by setting mime directly.
            let list_bytes = serde_json::to_vec(&json!({
                "schemaVersion": 2,
                "mediaType": "application/vnd.docker.distribution.manifest.list.v2+json",
                "manifests": [
                    {
                        "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
                        "size": 100,
                        "digest": "sha256:armdigest",
                        "platform": { "architecture": "arm64", "os": "linux" }
                    },
                    {
                        "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
                        "size": 100,
                        "digest": amd64_digest,
                        "platform": { "architecture": "amd64", "os": "linux" }
                    }
                ]
            }))
            .unwrap();

            let amd64_bytes = serde_json::to_vec(&json!({
                "schemaVersion": 2,
                "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
                "config": {
                    "mediaType": "application/vnd.docker.container.image.v1+json",
                    "size": 100,
                    "digest": "sha256:cfg"
                },
                "layers": []
            }))
            .unwrap();

            // First request: manifest list for "latest"
            Mock::given(method("GET"))
                .and(path("/v2/library/alpine/manifests/latest"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(
                    list_bytes,
                    "application/vnd.docker.distribution.manifest.list.v2+json",
                ))
                .expect(1)
                .mount(&server)
                .await;

            // Second request: resolved amd64 manifest — use path_regex since reqwest
            // may percent-encode ':' in "sha256:..." as "sha256%3A..." in the path.
            Mock::given(method("GET"))
                .and(path_regex(r"/manifests/sha256"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(
                    amd64_bytes,
                    "application/vnd.docker.distribution.manifest.v2+json",
                ))
                .expect(1)
                .mount(&server)
                .await;

            let manifest = test_client(&server)
                .get_manifest("library/alpine", "latest", "tok")
                .await
                .unwrap();

            // The resolved amd64 manifest has 0 layers
            assert_eq!(manifest.layers.len(), 0);
        }

        #[tokio::test]
        async fn get_manifest_errors_on_404() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v2/library/alpine/manifests/missing"))
                .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
                .mount(&server)
                .await;

            let err = test_client(&server)
                .get_manifest("library/alpine", "missing", "tok")
                .await
                .unwrap_err();
            let msg = err.to_string();
            assert!(msg.contains("HTTP 404"), "unexpected error: {msg}");
        }

        #[tokio::test]
        async fn get_manifest_errors_when_content_length_exceeds_limit() {
            let server = MockServer::start().await;
            // Send an actual body of MAX_MANIFEST_SIZE + 1 bytes — hyper validates
            // that Content-Length matches the real body, so we can't lie about it.
            // The client checks content-length before reading and rejects oversized.
            let oversized_body = vec![0u8; (MAX_MANIFEST_SIZE + 1) as usize];
            Mock::given(method("GET"))
                .and(path("/v2/library/alpine/manifests/latest"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/vnd.oci.image.manifest.v1+json")
                        .set_body_bytes(oversized_body),
                )
                .mount(&server)
                .await;

            let err = test_client(&server)
                .get_manifest("library/alpine", "latest", "tok")
                .await
                .unwrap_err();
            // Hits either the content-length header check or the streaming size check.
            assert!(
                err.to_string().contains("manifest too large")
                    || err.to_string().contains("manifest exceeded"),
                "unexpected error: {err}"
            );
        }

        #[tokio::test]
        async fn get_manifest_errors_when_no_amd64_in_list() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v2/library/alpine/manifests/latest"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_raw(
                        serde_json::to_vec(&json!({
                            "schemaVersion": 2,
                            "mediaType": "application/vnd.docker.distribution.manifest.list.v2+json",
                            "manifests": [
                                {
                                    "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
                                    "size": 100,
                                    "digest": "sha256:win",
                                    "platform": { "architecture": "amd64", "os": "windows" }
                                }
                            ]
                        }))
                        .unwrap(),
                        "application/vnd.docker.distribution.manifest.list.v2+json",
                    ),
                )
                .mount(&server)
                .await;

            let result = test_client(&server)
                .get_manifest("library/alpine", "latest", "tok")
                .await;
            assert!(result.is_err(), "should error when no linux/amd64 in list");
        }

        // ------------------------------------------------------------------
        // pull_layer
        // ------------------------------------------------------------------

        #[tokio::test]
        async fn pull_layer_returns_bytes_on_success() {
            let server = MockServer::start().await;
            let blob = b"fake layer blob data".to_vec();

            Mock::given(method("GET"))
                .and(path("/v2/library/alpine/blobs/sha256:fakedigest"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/octet-stream")
                        .set_body_bytes(blob.clone()),
                )
                .expect(1)
                .mount(&server)
                .await;

            let data = test_client(&server)
                .pull_layer("library/alpine", "sha256:fakedigest", "tok")
                .await
                .unwrap();
            assert_eq!(data.as_ref(), blob.as_slice());
        }

        #[tokio::test]
        async fn pull_layer_errors_on_404() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path_regex(r"^/v2/.*/blobs/.*$"))
                .respond_with(ResponseTemplate::new(404).set_body_string("blob not found"))
                .mount(&server)
                .await;

            let err = test_client(&server)
                .pull_layer("library/alpine", "sha256:missing", "tok")
                .await
                .unwrap_err();
            let msg = err.to_string();
            assert!(msg.contains("HTTP 404"), "unexpected error: {msg}");
        }

        // NOTE: pull_layer's Content-Length size check (MAX_LAYER_SIZE = 10 GiB) cannot
        // be exercised via wiremock — hyper requires Content-Length to match the actual
        // body size, making it infeasible to serve a 10 GiB body in a unit test.
        // The identical code pattern IS covered by get_manifest_errors_when_content_length_exceeds_limit.

        // ------------------------------------------------------------------
        // pull_image — end-to-end happy path
        // ------------------------------------------------------------------

        #[tokio::test]
        async fn pull_image_downloads_and_stores_all_layers() {
            let server = MockServer::start().await;
            let tmp = TempDir::new().unwrap();
            let store = ImageStore::new(tmp.path().join("images")).unwrap();
            let (layer_bytes, layer_digest) = make_test_layer();

            // Token endpoint
            Mock::given(method("GET"))
                .and(path("/token"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"token": "testtoken"})),
                )
                .expect(1)
                .mount(&server)
                .await;

            // Manifest endpoint
            Mock::given(method("GET"))
                .and(path("/v2/library/alpine/manifests/latest"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/vnd.oci.image.manifest.v1+json")
                        .set_body_json(json!({
                            "schemaVersion": 2,
                            "mediaType": "application/vnd.oci.image.manifest.v1+json",
                            "config": {
                                "mediaType": "application/vnd.oci.image.config.v1+json",
                                "size": 10,
                                "digest": "sha256:config"
                            },
                            "layers": [
                                {
                                    "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                                    "size": layer_bytes.len() as u64,
                                    "digest": layer_digest
                                }
                            ]
                        })),
                )
                .expect(1)
                .mount(&server)
                .await;

            // Blob endpoint — use path_regex since reqwest may percent-encode ':'
            // in "sha256:..." as "sha256%3A..." in the path segment.
            Mock::given(method("GET"))
                .and(path_regex(r"/blobs/sha256"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/octet-stream")
                        .set_body_bytes(layer_bytes),
                )
                .expect(1)
                .mount(&server)
                .await;

            test_client(&server)
                .pull_image("library/alpine", "latest", &store)
                .await
                .unwrap();

            assert!(
                store.has_image("library/alpine", "latest"),
                "image should be stored after pull_image"
            );
            let layers = store.get_image_layers("library/alpine", "latest").unwrap();
            assert_eq!(layers.len(), 1, "should have exactly one layer");
            assert!(layers[0].exists(), "layer directory should exist on disk");
        }

        #[tokio::test]
        async fn pull_image_errors_when_auth_fails() {
            let server = MockServer::start().await;
            let tmp = TempDir::new().unwrap();
            let store = ImageStore::new(tmp.path().join("images")).unwrap();

            Mock::given(method("GET"))
                .and(path("/token"))
                .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
                .expect(1)
                .mount(&server)
                .await;

            let err = test_client(&server)
                .pull_image("library/alpine", "latest", &store)
                .await
                .unwrap_err();
            let chain = format!("{err:#}");
            assert!(chain.contains("HTTP 401"), "unexpected error: {chain}");
        }

        #[tokio::test]
        async fn pull_image_errors_when_blob_fetch_fails() {
            let server = MockServer::start().await;
            let tmp = TempDir::new().unwrap();
            let store = ImageStore::new(tmp.path().join("images")).unwrap();
            let (layer_bytes, layer_digest) = make_test_layer();

            Mock::given(method("GET"))
                .and(path("/token"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"token": "testtoken"})),
                )
                .mount(&server)
                .await;

            Mock::given(method("GET"))
                .and(path("/v2/library/alpine/manifests/latest"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/vnd.oci.image.manifest.v1+json")
                        .set_body_json(json!({
                            "schemaVersion": 2,
                            "mediaType": "application/vnd.oci.image.manifest.v1+json",
                            "config": {
                                "mediaType": "application/vnd.oci.image.config.v1+json",
                                "size": 10,
                                "digest": "sha256:config"
                            },
                            "layers": [{
                                "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                                "size": layer_bytes.len() as u64,
                                "digest": layer_digest
                            }]
                        })),
                )
                .mount(&server)
                .await;

            Mock::given(method("GET"))
                .and(path_regex(r"/blobs/sha256"))
                .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
                .expect(1)
                .mount(&server)
                .await;

            let err = test_client(&server)
                .pull_image("library/alpine", "latest", &store)
                .await
                .unwrap_err();
            let chain = format!("{err:#}");
            assert!(chain.contains("HTTP 500"), "unexpected error: {chain}");
        }

        /// When a layer pull task fails, the layer's digest must appear in the
        /// error chain so callers can log or retry with the correct digest.
        /// Regression test for issue #151.
        #[tokio::test]
        async fn pull_failure_includes_digest() {
            let server = MockServer::start().await;
            let tmp = TempDir::new().expect("tmp dir");
            let store = ImageStore::new(tmp.path().join("images")).expect("image store");
            let (layer_bytes, layer_digest) = make_test_layer();

            Mock::given(method("GET"))
                .and(path("/token"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"token": "testtoken"})),
                )
                .mount(&server)
                .await;

            Mock::given(method("GET"))
                .and(path("/v2/library/alpine/manifests/latest"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/vnd.oci.image.manifest.v1+json")
                        .set_body_json(json!({
                            "schemaVersion": 2,
                            "mediaType": "application/vnd.oci.image.manifest.v1+json",
                            "config": {
                                "mediaType": "application/vnd.oci.image.config.v1+json",
                                "size": 10,
                                "digest": "sha256:config"
                            },
                            "layers": [{
                                "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                                "size": layer_bytes.len() as u64,
                                "digest": layer_digest
                            }]
                        })),
                )
                .mount(&server)
                .await;

            // Blob endpoint returns 500 — simulates a transient pull failure.
            Mock::given(method("GET"))
                .and(path_regex(r"/blobs/sha256"))
                .respond_with(ResponseTemplate::new(500).set_body_string("server error"))
                .expect(1)
                .mount(&server)
                .await;

            let err = test_client(&server)
                .pull_image("library/alpine", "latest", &store)
                .await
                .expect_err("pull_image should fail when blob fetch returns 500");

            let chain = format!("{err:#}");
            assert!(
                chain.contains(&layer_digest),
                "error chain must contain the layer digest ({layer_digest}) so callers can \
                 log/retry; got: {chain}"
            );
        }

        #[tokio::test]
        async fn pull_image_errors_on_digest_mismatch() {
            let server = MockServer::start().await;
            let tmp = TempDir::new().unwrap();
            let store = ImageStore::new(tmp.path().join("images")).unwrap();
            let (layer_bytes, layer_digest) = make_test_layer();

            Mock::given(method("GET"))
                .and(path("/token"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"token": "testtoken"})),
                )
                .mount(&server)
                .await;

            Mock::given(method("GET"))
                .and(path("/v2/library/alpine/manifests/latest"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/vnd.oci.image.manifest.v1+json")
                        .set_body_json(json!({
                            "schemaVersion": 2,
                            "mediaType": "application/vnd.oci.image.manifest.v1+json",
                            "config": {
                                "mediaType": "application/vnd.oci.image.config.v1+json",
                                "size": 10,
                                "digest": "sha256:config"
                            },
                            "layers": [{
                                "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                                "size": layer_bytes.len() as u64,
                                "digest": layer_digest  // correct digest in manifest...
                            }]
                        })),
                )
                .mount(&server)
                .await;

            // ...but serve corrupted bytes that won't match the digest
            Mock::given(method("GET"))
                .and(path_regex(r"/blobs/sha256"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/octet-stream")
                        .set_body_bytes(b"corrupted blob content".to_vec()),
                )
                .expect(1)
                .mount(&server)
                .await;

            let err = test_client(&server)
                .pull_image("library/alpine", "latest", &store)
                .await
                .unwrap_err();
            assert!(
                err.to_string().contains("digest"),
                "expected digest error, got: {err}"
            );
        }

        // ------------------------------------------------------------------
        // get_manifest — recursion depth guard
        // ------------------------------------------------------------------

        #[tokio::test]
        async fn get_manifest_errors_when_manifest_list_nesting_too_deep() {
            let server = MockServer::start().await;

            let list_bytes = |digest: &str| {
                serde_json::to_vec(&json!({
                    "schemaVersion": 2,
                    "mediaType": "application/vnd.docker.distribution.manifest.list.v2+json",
                    "manifests": [{
                        "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
                        "size": 100,
                        "digest": digest,
                        "platform": { "architecture": "amd64", "os": "linux" }
                    }]
                }))
                .unwrap()
            };

            // All paths return a manifest list — triggers the depth guard.
            Mock::given(method("GET"))
                .and(path_regex(r"/manifests/"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(
                    list_bytes("sha256:nested"),
                    "application/vnd.docker.distribution.manifest.list.v2+json",
                ))
                .mount(&server)
                .await;

            let err = test_client(&server)
                .get_manifest("library/alpine", "latest", "tok")
                .await
                .unwrap_err();
            assert!(
                err.to_string().contains("nesting too deep"),
                "expected nesting error, got: {err}"
            );
        }
    }

    // -------------------------------------------------------------------------
    // LimitedStream unit tests
    // -------------------------------------------------------------------------

    mod limited_stream {
        use super::super::LimitedStream;
        use bytes::Bytes;
        use futures::StreamExt;
        use futures::stream;

        fn bytes_stream(
            chunks: Vec<Vec<u8>>,
        ) -> impl futures::Stream<Item = Result<Bytes, std::io::Error>> {
            stream::iter(chunks.into_iter().map(|v| Ok(Bytes::from(v))))
        }

        #[tokio::test]
        async fn passes_chunks_under_limit() {
            let data: Vec<Vec<u8>> = vec![b"hello".to_vec(), b"world".to_vec()];
            let total = data.iter().map(|v| v.len()).sum::<usize>();
            let limit = (total + 100) as u64;
            let mut s = LimitedStream::new(bytes_stream(data), limit);
            let mut received = Vec::new();
            while let Some(chunk) = s.next().await {
                received.extend_from_slice(&chunk.unwrap());
            }
            assert_eq!(received, b"helloworld");
        }

        #[tokio::test]
        async fn errors_when_limit_exceeded() {
            let data: Vec<Vec<u8>> = vec![b"hello".to_vec(), b"world".to_vec()];
            // Limit of 4 bytes will be exceeded on first chunk
            let mut s = LimitedStream::new(bytes_stream(data), 4);
            // First chunk (5 bytes) should trigger error
            let result = s.next().await.unwrap();
            assert!(
                result.is_err(),
                "expected error when limit exceeded, got Ok"
            );
            let err = result.unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        }

        #[tokio::test]
        async fn tracks_consumed_bytes() {
            let data: Vec<Vec<u8>> = vec![b"abc".to_vec(), b"de".to_vec()];
            let mut s = LimitedStream::new(bytes_stream(data), 100);
            let _ = s.next().await;
            assert_eq!(s.consumed(), 3);
            let _ = s.next().await;
            assert_eq!(s.consumed(), 5);
        }

        // --- Boundary tests for #150 ---

        /// Exactly `limit` bytes must be allowed (consumed == limit is not an error).
        #[tokio::test]
        async fn exactly_limit_bytes_allowed() {
            // Single chunk of exactly 5 bytes; limit is also 5.
            let data: Vec<Vec<u8>> = vec![b"hello".to_vec()];
            let mut s = LimitedStream::new(bytes_stream(data), 5);
            let result = s.next().await.expect("stream should yield a chunk");
            assert!(
                result.is_ok(),
                "exactly limit bytes should succeed, got: {result:?}"
            );
            // Stream should be exhausted after the single chunk.
            assert!(s.next().await.is_none(), "stream should be exhausted");
        }

        /// `limit + 1` bytes must trigger an InvalidData error.
        #[tokio::test]
        async fn one_over_limit_errors() {
            // Single chunk of 6 bytes; limit is 5.
            let data: Vec<Vec<u8>> = vec![b"hello!".to_vec()];
            let mut s = LimitedStream::new(bytes_stream(data), 5);
            let result = s.next().await.expect("stream should yield an item");
            assert!(result.is_err(), "limit+1 bytes should return an error");
            assert_eq!(
                result.unwrap_err().kind(),
                std::io::ErrorKind::InvalidData,
                "error kind must be InvalidData"
            );
        }

        /// An error from the inner stream must be forwarded without a limit check.
        #[tokio::test]
        async fn inner_error_forwarded_before_limit_check() {
            use futures::stream;
            // stream::iter yields Unpin items; wrap an Err directly.
            let io_err = std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "simulated network reset",
            );
            let inner = stream::iter(vec![Err::<bytes::Bytes, std::io::Error>(io_err)]);
            let mut s = LimitedStream::new(inner, 1_000_000);
            let result = s.next().await.expect("should yield an error item");
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().kind(),
                std::io::ErrorKind::ConnectionReset
            );
        }

        /// Two chunks totalling exactly `limit` bytes across chunk boundaries.
        #[tokio::test]
        async fn boundary_split_across_chunks() {
            // Two 5-byte chunks; limit is 10.
            let data: Vec<Vec<u8>> = vec![b"hello".to_vec(), b"world".to_vec()];
            let mut s = LimitedStream::new(bytes_stream(data), 10);
            assert!(s.next().await.unwrap().is_ok(), "first chunk must pass");
            assert!(
                s.next().await.unwrap().is_ok(),
                "second chunk must pass (total == limit)"
            );
            assert!(s.next().await.is_none(), "stream must be exhausted");
        }

        // --- Tests added for #152 ---

        /// A limit of zero must reject the very first non-empty chunk.
        #[tokio::test]
        async fn zero_limit_rejects_first_byte() {
            let data: Vec<Vec<u8>> = vec![b"x".to_vec()];
            let mut s = LimitedStream::new(bytes_stream(data), 0);
            let result = s
                .next()
                .await
                .expect("stream should yield an item even when limit is zero");
            assert!(result.is_err(), "zero-limit stream must reject first byte");
            assert_eq!(
                result.unwrap_err().kind(),
                std::io::ErrorKind::InvalidData,
                "error kind must be InvalidData for zero-limit violation"
            );
        }

        /// A chunk that straddles the boundary (contains bytes both before and
        /// after the limit) must be rejected in its entirety — no partial
        /// forwarding of the permitted prefix.
        #[tokio::test]
        async fn chunk_straddles_boundary_rejected_whole() {
            // Limit is 3; first chunk has 2 bytes (ok), second has 3 bytes
            // which pushes consumed to 5 — one past the limit.
            let data: Vec<Vec<u8>> = vec![b"ab".to_vec(), b"cde".to_vec()];
            let mut s = LimitedStream::new(bytes_stream(data), 4);
            // First chunk (2 bytes, total 2) must pass.
            assert!(
                s.next().await.unwrap().is_ok(),
                "first chunk must pass (well under limit)"
            );
            // Second chunk (3 bytes, total 5 > 4) must be rejected wholesale.
            let result = s
                .next()
                .await
                .expect("stream must yield an item for the boundary-straddling chunk");
            assert!(
                result.is_err(),
                "chunk straddling the boundary must be rejected entirely"
            );
            assert_eq!(
                result.unwrap_err().kind(),
                std::io::ErrorKind::InvalidData,
                "error kind must be InvalidData"
            );
        }

        /// `consumed()` must include the bytes from the chunk that triggered
        /// the error — the rejected chunk is still counted.
        #[tokio::test]
        async fn consumed_reflects_rejected_chunk() {
            // Limit 3; single chunk of 5 bytes → error on first poll.
            let data: Vec<Vec<u8>> = vec![b"hello".to_vec()];
            let mut s = LimitedStream::new(bytes_stream(data), 3);
            let result = s.next().await.expect("stream must yield an item");
            assert!(result.is_err(), "limit must be exceeded");
            // consumed() must reflect the 5 bytes from the rejected chunk.
            assert_eq!(
                s.consumed(),
                5,
                "consumed() must include bytes from the rejected chunk"
            );
        }
    }

    // -------------------------------------------------------------------------
    // RegistryError::LayerTask digest propagation tests (#151)
    // -------------------------------------------------------------------------

    mod layer_task_digest {
        use crate::error::RegistryError;

        /// LayerTask error message must contain the digest, not "(unknown)".
        #[tokio::test]
        async fn layer_task_join_error_contains_digest() {
            let expected_digest = "sha256:deadbeefdeadbeef".to_owned();

            // Spawn a task that panics, then map the JoinError using the
            // captured digest — mirroring what pull_image does.
            let captured = expected_digest.clone();
            let join_result = tokio::spawn(async move {
                let _: () = panic!("simulated layer task panic");
            })
            .await;

            let err = RegistryError::LayerTask {
                digest: captured,
                source: join_result.unwrap_err(),
            };

            let msg = err.to_string();
            assert!(
                msg.contains("sha256:deadbeefdeadbeef"),
                "error message must contain the layer digest; got: {msg}"
            );
            assert!(
                !msg.contains("(unknown)"),
                "error message must not fall back to '(unknown)'; got: {msg}"
            );
        }
    }

    // -------------------------------------------------------------------------
    // Parallel pull failure model tests (#152)
    // -------------------------------------------------------------------------

    mod parallel_pull {
        use super::super::*;
        use crate::image::ImageStore;
        use flate2::{Compression, write::GzEncoder};
        use serde_json::json;
        use sha2::{Digest as ShaDigest, Sha256};
        use std::io::Write;
        use tempfile::TempDir;
        use wiremock::matchers::{method, path, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        fn test_client(server: &MockServer) -> RegistryClient {
            let mut client = RegistryClient::for_test(
                &format!("{}/token", server.uri()),
                &format!("{}/v2", server.uri()),
            )
            .expect("create test client");
            client.platform = crate::image::manifest::TargetPlatform::linux_amd64();
            client
        }

        fn make_test_layer() -> (Vec<u8>, String) {
            let data = b"minibox parallel pull test layer";
            let mut header = tar::Header::new_gnu();
            header.set_path("layer.txt").expect("set tar entry path");
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();

            let mut tar_buf = Vec::new();
            {
                let mut builder = tar::Builder::new(&mut tar_buf);
                builder
                    .append(&header, data.as_ref())
                    .expect("append tar entry");
                builder.finish().expect("finish tar archive");
            }

            let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
            gz.write_all(&tar_buf).expect("gz write");
            let bytes = gz.finish().expect("gz finish");
            let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
            (bytes, digest)
        }

        // ------------------------------------------------------------------
        // §3.1 / §1.4 — layers that completed before the first error persist
        // ------------------------------------------------------------------

        /// First layer is pre-cached on disk; second layer returns HTTP 500.
        /// After pull_image returns Err, the first layer directory must still
        /// exist (§1.4, §3.1).  Pre-caching layer 1 makes the test
        /// deterministic: the cached-layer early-exit fires before any network
        /// request, guaranteeing it is present when the second layer fails.
        #[tokio::test]
        async fn pull_image_second_layer_fails_first_cached() {
            let server = MockServer::start().await;
            let tmp = TempDir::new().expect("create tempdir");
            let store = ImageStore::new(tmp.path().join("images")).expect("create image store");

            let (layer1_bytes, layer1_digest) = make_test_layer();
            let layer2_digest =
                "sha256:0000000000000000000000000000000000000000000000000000000000000002";

            // Pre-create the layer1 directory so it is treated as cached.
            let layer1_key = layer1_digest.replace(':', "_");
            let layer1_dir = store
                .base_dir
                .join("library_alpine")
                .join("latest")
                .join("layers")
                .join(&layer1_key);
            std::fs::create_dir_all(&layer1_dir).expect("pre-create layer1 dir");

            // Token
            Mock::given(method("GET"))
                .and(path("/token"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": "tok"})))
                .mount(&server)
                .await;

            // Manifest with two layers
            Mock::given(method("GET"))
                .and(path("/v2/library/alpine/manifests/latest"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/vnd.oci.image.manifest.v1+json")
                        .set_body_json(json!({
                            "schemaVersion": 2,
                            "mediaType": "application/vnd.oci.image.manifest.v1+json",
                            "config": {
                                "mediaType": "application/vnd.oci.image.config.v1+json",
                                "size": 10,
                                "digest": "sha256:cfg"
                            },
                            "layers": [
                                {
                                    "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                                    "size": layer1_bytes.len() as u64,
                                    "digest": layer1_digest
                                },
                                {
                                    "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                                    "size": 999,
                                    "digest": layer2_digest
                                }
                            ]
                        })),
                )
                .mount(&server)
                .await;

            // Only layer 2 needs a network endpoint (layer 1 is cached).
            Mock::given(method("GET"))
                .and(path_regex(r"/blobs/sha256"))
                .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
                .mount(&server)
                .await;

            let err = test_client(&server)
                .pull_image("library/alpine", "latest", &store)
                .await
                .unwrap_err();
            let chain = format!("{err:#}");
            assert!(
                chain.contains("HTTP 500") || chain.contains("layer"),
                "unexpected error chain: {chain}"
            );

            // Layer 1 directory must still exist after the failure (§3.1).
            assert!(
                layer1_dir.exists(),
                "layer 1 dir must persist after second-layer failure: {layer1_dir:?}"
            );
        }

        // ------------------------------------------------------------------
        // §1.3 / §3.4 — all layers fail, no manifest stored
        // ------------------------------------------------------------------

        /// When all blobs return HTTP 500, pull_image returns an error and
        /// no manifest file is written (§1.3, §3.4).
        #[tokio::test]
        async fn pull_image_all_layers_fail() {
            let server = MockServer::start().await;
            let tmp = TempDir::new().expect("create tempdir");
            let store = ImageStore::new(tmp.path().join("images")).expect("create image store");
            let (layer_bytes, layer_digest) = make_test_layer();

            Mock::given(method("GET"))
                .and(path("/token"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": "tok"})))
                .mount(&server)
                .await;

            Mock::given(method("GET"))
                .and(path("/v2/library/alpine/manifests/latest"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/vnd.oci.image.manifest.v1+json")
                        .set_body_json(json!({
                            "schemaVersion": 2,
                            "mediaType": "application/vnd.oci.image.manifest.v1+json",
                            "config": {
                                "mediaType": "application/vnd.oci.image.config.v1+json",
                                "size": 10,
                                "digest": "sha256:cfg"
                            },
                            "layers": [{
                                "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                                "size": layer_bytes.len() as u64,
                                "digest": layer_digest
                            }]
                        })),
                )
                .mount(&server)
                .await;

            Mock::given(method("GET"))
                .and(path_regex(r"/blobs/sha256"))
                .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
                .mount(&server)
                .await;

            test_client(&server)
                .pull_image("library/alpine", "latest", &store)
                .await
                .unwrap_err();

            assert!(
                !store.has_image("library/alpine", "latest"),
                "manifest must not be stored when all layers fail"
            );
        }

        // ------------------------------------------------------------------
        // §3.4 / §1.1 — manifest not stored on layer failure
        // ------------------------------------------------------------------

        /// A single-layer pull that fails must leave `has_image` returning
        /// false — the manifest is written only after all layers succeed (§3.4).
        #[tokio::test]
        async fn pull_image_manifest_not_stored_on_layer_failure() {
            let server = MockServer::start().await;
            let tmp = TempDir::new().expect("create tempdir");
            let store = ImageStore::new(tmp.path().join("images")).expect("create image store");
            let (layer_bytes, layer_digest) = make_test_layer();

            Mock::given(method("GET"))
                .and(path("/token"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": "tok"})))
                .mount(&server)
                .await;

            Mock::given(method("GET"))
                .and(path("/v2/library/alpine/manifests/latest"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/vnd.oci.image.manifest.v1+json")
                        .set_body_json(json!({
                            "schemaVersion": 2,
                            "mediaType": "application/vnd.oci.image.manifest.v1+json",
                            "config": {
                                "mediaType": "application/vnd.oci.image.config.v1+json",
                                "size": 10,
                                "digest": "sha256:cfg"
                            },
                            "layers": [{
                                "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                                "size": layer_bytes.len() as u64,
                                "digest": layer_digest
                            }]
                        })),
                )
                .mount(&server)
                .await;

            Mock::given(method("GET"))
                .and(path_regex(r"/blobs/sha256"))
                .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
                .mount(&server)
                .await;

            let _ = test_client(&server)
                .pull_image("library/alpine", "latest", &store)
                .await;

            assert!(
                !store.has_image("library/alpine", "latest"),
                "has_image must return false when layer pull failed"
            );
            // Verify the manifest file itself is absent
            let manifest_path = store
                .base_dir
                .join("library_alpine")
                .join("latest")
                .join("manifest.json");
            assert!(
                !manifest_path.exists(),
                "manifest.json must not exist after failed pull: {manifest_path:?}"
            );
        }

        // ------------------------------------------------------------------
        // §4.3 — cached layers are skipped on re-pull
        // ------------------------------------------------------------------

        /// Running pull_image twice should only fetch each blob once; the
        /// second pull finds the layer directory already present and skips it.
        #[tokio::test]
        async fn pull_image_skips_cached_layers_on_repull() {
            let server = MockServer::start().await;
            let tmp = TempDir::new().expect("create tempdir");
            let store = ImageStore::new(tmp.path().join("images")).expect("create image store");
            let (layer_bytes, layer_digest) = make_test_layer();

            Mock::given(method("GET"))
                .and(path("/token"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": "tok"})))
                .mount(&server)
                .await;

            Mock::given(method("GET"))
                .and(path("/v2/library/alpine/manifests/latest"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/vnd.oci.image.manifest.v1+json")
                        .set_body_json(json!({
                            "schemaVersion": 2,
                            "mediaType": "application/vnd.oci.image.manifest.v1+json",
                            "config": {
                                "mediaType": "application/vnd.oci.image.config.v1+json",
                                "size": 10,
                                "digest": "sha256:cfg"
                            },
                            "layers": [{
                                "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                                "size": layer_bytes.len() as u64,
                                "digest": layer_digest
                            }]
                        })),
                )
                .mount(&server)
                .await;

            // Blob endpoint — expect exactly ONE call across both pull_image invocations.
            Mock::given(method("GET"))
                .and(path_regex(r"/blobs/sha256"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/octet-stream")
                        .set_body_bytes(layer_bytes),
                )
                .expect(1) // second pull must hit the cache, not the network
                .mount(&server)
                .await;

            let client = test_client(&server);
            client
                .pull_image("library/alpine", "latest", &store)
                .await
                .expect("first pull must succeed");
            client
                .pull_image("library/alpine", "latest", &store)
                .await
                .expect("second pull must succeed (from cache)");

            // wiremock will assert the `.expect(1)` on drop
        }

        // ------------------------------------------------------------------
        // §4.3 — stale tmp dir is removed before extraction
        // ------------------------------------------------------------------

        /// If a `*.tmp` directory from a previous failed pull is present on
        /// disk when pull_image runs, it must be removed before the new
        /// extraction begins (§4.3).
        #[tokio::test]
        async fn pull_image_stale_tmp_dir_removed_on_repull() {
            let server = MockServer::start().await;
            let tmp = TempDir::new().expect("create tempdir");
            let store = ImageStore::new(tmp.path().join("images")).expect("create image store");
            let (layer_bytes, layer_digest) = make_test_layer();

            Mock::given(method("GET"))
                .and(path("/token"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": "tok"})))
                .mount(&server)
                .await;

            Mock::given(method("GET"))
                .and(path("/v2/library/alpine/manifests/latest"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/vnd.oci.image.manifest.v1+json")
                        .set_body_json(json!({
                            "schemaVersion": 2,
                            "mediaType": "application/vnd.oci.image.manifest.v1+json",
                            "config": {
                                "mediaType": "application/vnd.oci.image.config.v1+json",
                                "size": 10,
                                "digest": "sha256:cfg"
                            },
                            "layers": [{
                                "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                                "size": layer_bytes.len() as u64,
                                "digest": layer_digest
                            }]
                        })),
                )
                .mount(&server)
                .await;

            Mock::given(method("GET"))
                .and(path_regex(r"/blobs/sha256"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/octet-stream")
                        .set_body_bytes(layer_bytes),
                )
                .mount(&server)
                .await;

            // Manually create a stale tmp directory where pull_image expects it.
            let digest_key = layer_digest.replace(':', "_");
            let stale_tmp = store
                .base_dir
                .join("library_alpine")
                .join("latest")
                .join("layers")
                .join(format!("{digest_key}.tmp"));
            std::fs::create_dir_all(&stale_tmp).expect("create stale tmp dir");
            // Add a sentinel file so we can verify it was replaced.
            std::fs::write(stale_tmp.join("stale_marker"), b"old").expect("write stale marker");

            test_client(&server)
                .pull_image("library/alpine", "latest", &store)
                .await
                .expect("pull must succeed despite stale tmp dir");

            // The layer dir (not tmp) must now exist with the fresh content.
            let layer_dir = store
                .base_dir
                .join("library_alpine")
                .join("latest")
                .join("layers")
                .join(&digest_key);
            assert!(
                layer_dir.exists(),
                "layer dir must exist after successful pull: {layer_dir:?}"
            );
            // The stale marker file must be gone (tmp was removed and recreated).
            assert!(
                !layer_dir.join("stale_marker").exists(),
                "stale marker file must not appear in the final layer dir"
            );
        }
    }

    #[test]
    fn test_registry_client_new_has_default_platform() {
        let client = RegistryClient::new().expect("should create client");
        let default_tp = crate::image::manifest::TargetPlatform::default();
        assert_eq!(client.platform, default_tp);
    }

    #[test]
    fn test_registry_client_with_platform() {
        let tp =
            crate::image::manifest::TargetPlatform::parse("linux/arm64/v8").expect("should parse");
        let client = RegistryClient::with_platform(tp.clone()).expect("should create client");
        assert_eq!(client.platform, tp);
    }
}
