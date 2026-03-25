//! Docker Hub / OCI registry client.
//!
//! Supports anonymous token authentication (sufficient for public images) and
//! pulls manifests and blobs from `registry-1.docker.io`.
//!
//! # Usage
//!
//! ```rust,no_run
//! use linuxbox::image::{ImageStore, registry::RegistryClient};
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

// SECURITY: Resource limits to prevent DoS attacks.
// MAX_MANIFEST_SIZE: manifests are small JSON blobs; 10 MB is a generous ceiling.
// MAX_LAYER_SIZE: individual compressed layer blobs; 10 GB allows large images while
// bounding memory consumption during streaming download.
const MAX_MANIFEST_SIZE: u64 = 10 * 1024 * 1024; // 10 MiB
const MAX_LAYER_SIZE: u64 = 10 * 1024 * 1024 * 1024; // 10 GiB

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
    auth_url: String,
    registry_base: String,
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
        Ok(Self {
            http,
            auth_url: AUTH_URL.to_owned(),
            registry_base: REGISTRY_BASE.to_owned(),
        })
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
            http,
            auth_url: auth_url.to_owned(),
            registry_base: registry_base.to_owned(),
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
                let amd64 = list
                    .find_linux_amd64()
                    .ok_or(RegistryError::NoAmd64Manifest)?;
                info!("manifest list resolved to amd64 digest={}", amd64.digest);
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

    /// Download a single blob by `digest` and return its raw bytes.
    ///
    /// Docker Hub redirects blob requests to a CDN; the client follows these
    /// redirects automatically.
    #[instrument(skip(self, token), fields(digest = %digest.get(..19).unwrap_or(digest)))]
    pub async fn pull_layer(&self, name: &str, digest: &str, token: &str) -> anyhow::Result<Bytes> {
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

            let digest_short = layer_desc.digest.get(..19).unwrap_or(&layer_desc.digest);

            if layer_dir.exists() {
                info!(
                    "layer {}/{}: {} (cached)",
                    idx + 1,
                    manifest.layers.len(),
                    digest_short
                );
                continue;
            }

            let layer_span = tracing::info_span!(
                "layer",
                n = idx + 1,
                total = manifest.layers.len(),
                digest = digest_short,
            );

            let layer_start = std::time::Instant::now();
            let download_start = std::time::Instant::now();
            let data = self
                .pull_layer(name, &layer_desc.digest, &token)
                .instrument(layer_span.clone())
                .await
                .with_context(|| format!("pull layer {}", layer_desc.digest))?;
            let download = download_start.elapsed();

            {
                let _guard = layer_span.enter();

                let verify_start = std::time::Instant::now();
                {
                    let _span = tracing::debug_span!("verify_digest").entered();
                    verify_digest(&data, &layer_desc.digest).with_context(|| {
                        format!("digest verification for {}", layer_desc.digest)
                    })?;
                }
                let verify = verify_start.elapsed();

                let extract_start = std::time::Instant::now();
                {
                    let _span = tracing::debug_span!("extract", bytes = data.len()).entered();
                    store
                        .store_layer(name, tag, &layer_desc.digest, std::io::Cursor::new(data))
                        .with_context(|| format!("store layer {}", layer_desc.digest))?;
                }
                let extract = extract_start.elapsed();

                info!(
                    "layer {}/{} ({}) done in {:.2?} — download {:.2?} verify {:.2?} extract {:.2?}",
                    idx + 1,
                    manifest.layers.len(),
                    digest_short,
                    layer_start.elapsed(),
                    download,
                    verify,
                    extract,
                );
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
            RegistryClient::for_test(
                &format!("{}/token", server.uri()),
                &format!("{}/v2", server.uri()),
            )
            .unwrap()
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
}
