//! GitHub Container Registry (ghcr.io) adapter implementing the ImageRegistry trait.
//!
//! Authenticates via `WWW-Authenticate` Bearer challenge. Pass a personal access
//! token (PAT) with `read:packages` scope as `GHCR_TOKEN` to access private images;
//! public images work without a token.

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::TryStreamExt;
use minibox_core::as_any;
use minibox_core::domain::{ImageMetadata, ImageRegistry, LayerInfo};
use minibox_core::image::ImageStore;
use minibox_core::image::manifest::{ManifestResponse, TargetPlatform};
use serde::Deserialize;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio_util::io::{StreamReader, SyncIoBridge};
use tracing::info;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const GHCR_BASE: &str = "https://ghcr.io/v2";
const MAX_MANIFEST_SIZE: u64 = 10 * 1024 * 1024; // 10 MB
const MAX_LAYER_SIZE: u64 = 10 * 1024 * 1024 * 1024; // 10 GB

const ACCEPT_MANIFESTS: &str = concat!(
    "application/vnd.oci.image.manifest.v1+json, ",
    "application/vnd.oci.image.index.v1+json, ",
    "application/vnd.docker.distribution.manifest.v2+json, ",
    "application/vnd.docker.distribution.manifest.list.v2+json"
);

// ---------------------------------------------------------------------------
// Auth token response
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
}

// ---------------------------------------------------------------------------
// GhcrRegistry
// ---------------------------------------------------------------------------

/// GitHub Container Registry implementation of the [`ImageRegistry`] trait.
///
/// Authenticates via the OCI Distribution Spec `WWW-Authenticate` Bearer
/// challenge. Set `GHCR_TOKEN` to a PAT with `read:packages` to pull private
/// images; public images work without a token.
///
/// Images are stored under `ghcr.io/<name>` in the local [`ImageStore`] to
/// avoid collisions with Docker Hub images.
pub struct GhcrRegistry {
    store: Arc<ImageStore>,
    token: Option<String>, // GHCR_TOKEN env var
    http: reqwest::Client,
    base_url: String,
    /// Per-request platform override; falls back to [`TargetPlatform::default`] when `None`.
    platform: Option<TargetPlatform>,
}

impl GhcrRegistry {
    /// Create a new GHCR adapter.
    ///
    /// Reads `GHCR_TOKEN` from the environment if present.
    pub fn new(store: Arc<ImageStore>) -> Result<Self> {
        let token = std::env::var("GHCR_TOKEN").ok();
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .https_only(true)
            .min_tls_version(reqwest::tls::Version::TLS_1_2)
            .build()?;
        Ok(Self {
            store,
            token,
            http,
            base_url: GHCR_BASE.to_owned(),
            platform: None,
        })
    }

    /// Create a GHCR adapter targeting a specific platform.
    ///
    /// Use this when pulling multi-arch images for a non-host architecture.
    /// The platform selects which manifest entry to resolve from a manifest list.
    pub fn with_platform(store: Arc<ImageStore>, platform: TargetPlatform) -> Result<Self> {
        let mut registry = Self::new(store)?;
        registry.platform = Some(platform);
        Ok(registry)
    }

    /// Return the effective [`TargetPlatform`] for manifest list resolution.
    fn effective_platform(&self) -> TargetPlatform {
        self.platform.clone().unwrap_or_default()
    }

    /// Create a test adapter pointed at a plain-HTTP mock server.
    #[cfg(test)]
    fn for_test(store: Arc<ImageStore>, base_url: &str, token: Option<&str>) -> Self {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .expect("test reqwest client");
        Self {
            store,
            token: token.map(str::to_owned),
            http,
            base_url: base_url.to_owned(),
            platform: None,
        }
    }

    // -----------------------------------------------------------------------
    // Authentication
    // -----------------------------------------------------------------------

    /// Obtain a Bearer token for `repo`/`tag` via the `WWW-Authenticate` challenge.
    ///
    /// Probes the manifest for the actual requested tag so that repos without a
    /// `latest` tag can still authenticate. Returns an empty string for public images.
    async fn authenticate(&self, repo: &str, tag: &str) -> Result<String> {
        // Probe with the real tag — repos that don't publish `latest` return 404
        // without a WWW-Authenticate header, breaking token exchange.
        let url = format!("{}/{repo}/manifests/{tag}", self.base_url);
        let resp = self.http.get(&url).send().await?;

        if resp.status() == reqwest::StatusCode::OK {
            // Public image, no auth needed.
            return Ok(String::new());
        }

        // Parse the WWW-Authenticate challenge.
        let www_auth = resp
            .headers()
            .get("WWW-Authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();

        let (realm, service, scope) = parse_www_authenticate(&www_auth);
        if realm.is_empty() {
            anyhow::bail!("ghcr: no WWW-Authenticate realm for {repo}");
        }

        // Exchange for a token, optionally using the caller-supplied PAT.
        let mut req = self
            .http
            .get(&realm)
            .query(&[("service", &service), ("scope", &scope)]);
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }

        let token_resp: TokenResponse = req
            .send()
            .await
            .with_context(|| format!("ghcr: token request for {repo}"))?
            .json()
            .await
            .with_context(|| format!("ghcr: parsing token response for {repo}"))?;

        Ok(token_resp.token)
    }

    // -----------------------------------------------------------------------
    // Manifest
    // -----------------------------------------------------------------------

    /// Fetch the manifest for `repo` at `tag_or_digest`.
    ///
    /// Manifest lists are resolved to the `linux/amd64` entry automatically.
    async fn get_manifest(
        &self,
        repo: &str,
        tag_or_digest: &str,
        token: &str,
    ) -> Result<crate::image::manifest::OciManifest> {
        let url = format!("{}/{repo}/manifests/{tag_or_digest}", self.base_url);
        let mut req = self.http.get(&url).header("Accept", ACCEPT_MANIFESTS);
        if !token.is_empty() {
            req = req.bearer_auth(token);
        }

        let resp = req
            .send()
            .await
            .with_context(|| format!("ghcr: GET manifest {repo}:{tag_or_digest}"))?;

        anyhow::ensure!(
            resp.status().is_success(),
            "ghcr: manifest fetch failed: {} for {repo}:{tag_or_digest}",
            resp.status(),
        );

        // SECURITY: Check Content-Length before buffering.
        if let Some(cl) = resp.headers().get("content-length")
            && let Ok(s) = cl.to_str()
            && let Ok(n) = s.parse::<u64>()
            && n > MAX_MANIFEST_SIZE
        {
            anyhow::bail!("ghcr: manifest too large: {n} bytes (max {MAX_MANIFEST_SIZE})");
        }

        let content_type = resp
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();

        let bytes = resp
            .bytes()
            .await
            .with_context(|| format!("ghcr: reading manifest body for {repo}:{tag_or_digest}"))?;

        anyhow::ensure!(
            bytes.len() as u64 <= MAX_MANIFEST_SIZE,
            "ghcr: manifest body exceeded size limit for {repo}:{tag_or_digest}",
        );

        let manifest_resp = ManifestResponse::parse(&bytes, &content_type)
            .with_context(|| format!("ghcr: parsing manifest for {repo}:{tag_or_digest}"))?;

        match manifest_resp {
            ManifestResponse::Single(m) => Ok(m),
            ManifestResponse::List(list) => {
                let platform = self.effective_platform();
                let desc = list.find_platform(&platform).ok_or_else(|| {
                    anyhow::anyhow!(
                        "ghcr: no {platform} manifest in list for {repo}:{tag_or_digest}",
                    )
                })?;
                let digest = desc.digest.clone();
                Box::pin(self.get_manifest(repo, &digest, token)).await
            }
        }
    }

    // -----------------------------------------------------------------------
    // Blob / layer pull
    // -----------------------------------------------------------------------

    /// Fetch a single blob by `digest` and return the streaming response.
    ///
    /// The Content-Length is checked against `MAX_LAYER_SIZE` before returning.
    /// The caller is responsible for streaming the body to disk.
    async fn pull_layer(&self, repo: &str, digest: &str, token: &str) -> Result<reqwest::Response> {
        let url = format!("{}/{repo}/blobs/{digest}", self.base_url);
        let mut req = self.http.get(&url);
        if !token.is_empty() {
            req = req.bearer_auth(token);
        }

        let resp = req
            .send()
            .await
            .with_context(|| format!("ghcr: GET blob {digest}"))?;

        anyhow::ensure!(
            resp.status().is_success(),
            "ghcr: blob fetch failed: {} for {digest}",
            resp.status(),
        );

        // SECURITY: Reject oversized layers before streaming begins.
        if let Some(cl) = resp.headers().get("content-length")
            && let Ok(s) = cl.to_str()
            && let Ok(n) = s.parse::<u64>()
            && n > MAX_LAYER_SIZE
        {
            anyhow::bail!("ghcr: layer too large: {n} bytes (max {MAX_LAYER_SIZE})");
        }

        Ok(resp)
    }
}

// ---------------------------------------------------------------------------
// Allowlist enforcement
// ---------------------------------------------------------------------------

/// Check `repo` (e.g. `"org/image"`) against the `GHCR_ORG_ALLOWLIST` env var.
///
/// The variable is a comma-separated list of org or `org/repo` prefixes.
/// If the variable is unset, all repositories are permitted.
/// If it is set, the pull is rejected unless `repo` equals a listed prefix
/// or starts with one followed by `/`.
///
/// Example: `GHCR_ORG_ALLOWLIST=myorg,myorg/private-image`
fn check_ghcr_allowlist(repo: &str) -> Result<()> {
    let Ok(list) = std::env::var("GHCR_ORG_ALLOWLIST") else {
        return Ok(()); // no allowlist configured → allow all
    };
    let permitted = list
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|prefix| repo == prefix || repo.starts_with(&format!("{prefix}/")));
    if permitted {
        Ok(())
    } else {
        anyhow::bail!(
            "ghcr: repository {repo:?} is not in GHCR_ORG_ALLOWLIST ({list:?}); \
             set GHCR_ORG_ALLOWLIST to include this org/repo or unset it to allow all"
        )
    }
}

as_any!(GhcrRegistry);

// ---------------------------------------------------------------------------
// ImageRegistry trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ImageRegistry for GhcrRegistry {
    async fn has_image(&self, name: &str, tag: &str) -> bool {
        // Callers pass image_ref.cache_name() which is already fully-qualified
        // (e.g. "ghcr.io/org/image"), so use it directly.
        self.store.has_image(name, tag)
    }

    async fn pull_image(
        &self,
        image_ref: &crate::image::reference::ImageRef,
    ) -> Result<ImageMetadata> {
        let store_key = image_ref.cache_name(); // "ghcr.io/org/image"
        let repo = image_ref.repository(); // "org/image" (for ghcr.io API path)
        let tag = &image_ref.tag;

        check_ghcr_allowlist(&repo).with_context(|| format!("ghcr: allowlist check for {repo}"))?;

        info!("ghcr: pulling {store_key}:{tag}");

        let token = self
            .authenticate(&repo, tag)
            .await
            .with_context(|| format!("ghcr: authenticate for {repo}:{tag}"))?;

        let manifest = self
            .get_manifest(&repo, tag, &token)
            .await
            .with_context(|| format!("ghcr: get manifest for {repo}:{tag}"))?;

        let mut layer_infos = Vec::new();
        for layer in &manifest.layers {
            let digest = &layer.digest;
            let resp = self
                .pull_layer(&repo, digest, &token)
                .await
                .with_context(|| format!("ghcr: pull layer {digest}"))?;

            // Stream the blob body through a size-limited reader, then into
            // verified layer storage (HashingReader + tmp dir + digest check +
            // atomic rename).
            let stream = resp.bytes_stream().map_err(io::Error::other);
            let async_reader = StreamReader::new(stream);
            // Capture the runtime handle before entering spawn_blocking -- inside
            // a blocking thread there is no Tokio context to call Handle::current().
            let handle = Handle::current();
            let store = Arc::clone(&self.store);
            let store_key2 = store_key.clone();
            let tag2 = tag.to_owned();
            let digest2 = digest.clone();
            tokio::task::spawn_blocking(move || {
                let sync_reader = SyncIoBridge::new_with_handle(async_reader, handle);
                store
                    .store_layer_verified(&store_key2, &tag2, &digest2, sync_reader)
                    .with_context(|| format!("ghcr: store layer {digest2}"))
            })
            .await
            .context("ghcr: spawn_blocking for store_layer")??;

            layer_infos.push(LayerInfo {
                digest: layer.digest.clone(),
                size: layer.size,
            });
        }

        self.store
            .store_manifest(&store_key, tag, &manifest)
            .with_context(|| format!("ghcr: store manifest for {store_key}:{tag}"))?;

        let n = layer_infos.len();
        info!("ghcr: pulled {store_key}:{tag} ({n} layers)");

        Ok(ImageMetadata {
            name: store_key,
            tag: tag.to_owned(),
            layers: layer_infos,
        })
    }

    fn get_image_layers(&self, name: &str, tag: &str) -> Result<Vec<PathBuf>> {
        // Callers pass image_ref.cache_name() which is already fully-qualified.
        self.store.get_image_layers(name, tag)
    }
}

// ---------------------------------------------------------------------------
// WWW-Authenticate header parser
// ---------------------------------------------------------------------------

/// Parse a `WWW-Authenticate: Bearer ...` header into `(realm, service, scope)`.
///
/// Returns empty strings for any missing fields.
pub fn parse_www_authenticate(header: &str) -> (String, String, String) {
    let mut realm = String::new();
    let mut service = String::new();
    let mut scope = String::new();

    // Strip the "Bearer " prefix if present.
    let body = header.strip_prefix("Bearer ").unwrap_or(header);

    let mut remaining = body;
    while !remaining.is_empty() {
        // Find key=value separator.
        let eq = match remaining.find('=') {
            Some(i) => i,
            None => break,
        };
        let key = remaining[..eq].trim();
        remaining = &remaining[eq + 1..];

        // Parse the (possibly quoted) value.
        let value = if remaining.starts_with('"') {
            remaining = &remaining[1..];
            match remaining.find('"') {
                Some(end) => {
                    let val = remaining[..end].to_owned();
                    remaining = &remaining[end + 1..];
                    if remaining.starts_with(',') {
                        remaining = &remaining[1..];
                    }
                    val
                }
                None => {
                    let val = remaining.to_owned();
                    remaining = "";
                    val
                }
            }
        } else {
            match remaining.find(',') {
                Some(end) => {
                    let val = remaining[..end].to_owned();
                    remaining = &remaining[end + 1..];
                    val
                }
                None => {
                    let val = remaining.to_owned();
                    remaining = "";
                    val
                }
            }
        };

        match key {
            "realm" => realm = value,
            "service" => service = value,
            "scope" => scope = value,
            _ => {}
        }
    }

    (realm, service, scope)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn parse_www_authenticate_full() {
        let header = r#"Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:org/image:pull""#;
        let (realm, service, scope) = parse_www_authenticate(header);
        assert_eq!(realm, "https://ghcr.io/token");
        assert_eq!(service, "ghcr.io");
        assert_eq!(scope, "repository:org/image:pull");
    }

    #[test]
    fn parse_www_authenticate_missing_service() {
        let header = r#"Bearer realm="https://ghcr.io/token",scope="repository:org/image:pull""#;
        let (realm, service, scope) = parse_www_authenticate(header);
        assert_eq!(realm, "https://ghcr.io/token");
        assert_eq!(service, "");
        assert_eq!(scope, "repository:org/image:pull");
    }

    #[test]
    fn parse_www_authenticate_empty() {
        let (realm, service, scope) = parse_www_authenticate("");
        assert_eq!(realm, "");
        assert_eq!(service, "");
        assert_eq!(scope, "");
    }

    // Mutex to serialise GHCR_ORG_ALLOWLIST mutation across parallel tests.
    static ALLOWLIST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn allowlist_permits_when_unset() {
        let _g = ALLOWLIST_ENV_LOCK.lock().unwrap();
        // SAFETY: single-threaded section guarded by ALLOWLIST_ENV_LOCK.
        unsafe { std::env::remove_var("GHCR_ORG_ALLOWLIST") };
        assert!(check_ghcr_allowlist("org/image").is_ok());
    }

    #[test]
    fn allowlist_permits_matching_org() {
        let _g = ALLOWLIST_ENV_LOCK.lock().unwrap();
        // SAFETY: single-threaded section guarded by ALLOWLIST_ENV_LOCK.
        unsafe { std::env::set_var("GHCR_ORG_ALLOWLIST", "myorg,otherorg") };
        assert!(check_ghcr_allowlist("myorg/image").is_ok());
        assert!(check_ghcr_allowlist("otherorg/tool").is_ok());
        unsafe { std::env::remove_var("GHCR_ORG_ALLOWLIST") };
    }

    #[test]
    fn allowlist_rejects_unlisted_org() {
        let _g = ALLOWLIST_ENV_LOCK.lock().unwrap();
        // SAFETY: single-threaded section guarded by ALLOWLIST_ENV_LOCK.
        unsafe { std::env::set_var("GHCR_ORG_ALLOWLIST", "allowedorg") };
        assert!(check_ghcr_allowlist("otherog/img").is_err());
        unsafe { std::env::remove_var("GHCR_ORG_ALLOWLIST") };
    }

    #[test]
    fn ghcr_registry_constructs() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(dir.path().join("images")).unwrap());
        let reg = GhcrRegistry::new(store);
        assert!(reg.is_ok());
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
        use wiremock::matchers::{header, method, path, path_regex, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        fn make_store(dir: &TempDir) -> Arc<ImageStore> {
            Arc::new(ImageStore::new(dir.path().join("images")).expect("ImageStore"))
        }

        /// Build a minimal gzip-compressed tar layer; return (bytes, sha256-digest).
        fn make_test_layer() -> (Vec<u8>, String) {
            let data = b"ghcr test layer";
            let mut header = tar::Header::new_gnu();
            header.set_path("hello.txt").expect("set path");
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();

            let mut tar_buf = Vec::new();
            {
                let mut builder = tar::Builder::new(&mut tar_buf);
                builder.append(&header, data.as_ref()).expect("append");
                builder.finish().expect("finish");
            }
            let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
            gz.write_all(&tar_buf).expect("gz write");
            let bytes = gz.finish().expect("gz finish");
            let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
            (bytes, digest)
        }

        /// OCI manifest JSON for a single layer.
        fn manifest_json(digest: &str, size: usize) -> serde_json::Value {
            json!({
                "schemaVersion": 2,
                "mediaType": "application/vnd.oci.image.manifest.v1+json",
                "config": {
                    "mediaType": "application/vnd.oci.image.config.v1+json",
                    "digest": "sha256:abc",
                    "size": 100
                },
                "layers": [{
                    "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                    "digest": digest,
                    "size": size
                }]
            })
        }

        /// Mount a complete auth + manifest flow for `repo`/`tag`.
        ///
        /// wiremock uses FIFO matching (first registered = highest priority).
        /// Registration order:
        ///
        /// 1. 200 manifest mock with `Authorization: Bearer {tok}` (FIRST = highest
        ///    priority) — only matches authenticated requests.
        /// 2. Token exchange mock.
        /// 3. 401 mock (LAST = lowest priority) — fallback for the unauthenticated
        ///    probe which lacks the Authorization header and skips mock 1.
        async fn mount_auth_with_manifest(
            server: &MockServer,
            repo: &str,
            tag: &str,
            tok: &str,
            manifest_body: Vec<u8>,
        ) {
            let manifest_path = format!("/v2/{repo}/manifests/{tag}");
            let www_auth = format!(
                r#"Bearer realm="{}/token",service="ghcr.io",scope="repository:{repo}:pull""#,
                server.uri(),
            );
            // Step 1 (FIFO highest priority): authenticated manifest fetch.
            // Only matches requests that carry Authorization: Bearer <tok>.
            Mock::given(method("GET"))
                .and(path(&manifest_path))
                .and(header("Authorization", format!("Bearer {tok}").as_str()))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(manifest_body, "application/vnd.oci.image.manifest.v1+json"),
                )
                .expect(1)
                .mount(server)
                .await;
            // Step 2: token exchange.
            Mock::given(method("GET"))
                .and(path("/token"))
                .and(query_param("service", "ghcr.io"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "token": tok })))
                .expect(1)
                .mount(server)
                .await;
            // Step 3 (FIFO lowest priority): unauthenticated probe → 401.
            // The probe has no Authorization header so it skips mock 1 and lands here.
            Mock::given(method("GET"))
                .and(path(&manifest_path))
                .respond_with(
                    ResponseTemplate::new(401).insert_header("WWW-Authenticate", www_auth.as_str()),
                )
                .expect(1)
                .mount(server)
                .await;
        }

        // ------------------------------------------------------------------
        // Cache hit / miss
        // ------------------------------------------------------------------

        #[tokio::test]
        async fn has_image_returns_false_for_empty_store() {
            let dir = TempDir::new().unwrap();
            let store = make_store(&dir);
            let reg = GhcrRegistry::for_test(store, "http://unused", None);
            assert!(!reg.has_image("ghcr.io/org/image", "latest").await);
        }

        #[tokio::test]
        async fn has_image_returns_true_after_pull() {
            let dir = TempDir::new().unwrap();
            let store = make_store(&dir);
            let server = MockServer::start().await;
            let base = format!("{}/v2", server.uri());
            let (layer_bytes, digest) = make_test_layer();
            let size = layer_bytes.len();

            let manifest = serde_json::to_vec(&manifest_json(&digest, size)).unwrap();
            mount_auth_with_manifest(&server, "org/img", "v1.0", "tok", manifest).await;

            Mock::given(method("GET"))
                .and(path_regex(r"^/v2/org/img/blobs/sha256:.*"))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(layer_bytes))
                .expect(1)
                .mount(&server)
                .await;

            let reg = GhcrRegistry::for_test(Arc::clone(&store), &base, None);
            let image_ref =
                crate::image::reference::ImageRef::parse("ghcr.io/org/img:v1.0").expect("ref");
            reg.pull_image(&image_ref).await.expect("pull_image");

            assert!(reg.has_image("ghcr.io/org/img", "v1.0").await);
        }

        // ------------------------------------------------------------------
        // Auth: versioned tag, no `latest`
        // ------------------------------------------------------------------

        #[tokio::test]
        async fn authenticate_succeeds_with_versioned_tag_no_latest() {
            let dir = TempDir::new().unwrap();
            let store = make_store(&dir);
            let server = MockServer::start().await;
            let base = format!("{}/v2", server.uri());

            // The repo has no `latest` — only `v2.3.1`.
            // authenticate() must probe /manifests/v2.3.1, not /manifests/latest.
            let www_auth = format!(
                r#"Bearer realm="{}/token",service="ghcr.io",scope="repository:org/versioned:pull""#,
                server.uri(),
            );
            Mock::given(method("GET"))
                .and(path("/v2/org/versioned/manifests/v2.3.1"))
                .respond_with(
                    ResponseTemplate::new(401).insert_header("WWW-Authenticate", www_auth.as_str()),
                )
                .expect(1)
                .mount(&server)
                .await;

            Mock::given(method("GET"))
                .and(path("/token"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({ "token": "versioned_tok" })),
                )
                .expect(1)
                .mount(&server)
                .await;

            let reg = GhcrRegistry::for_test(store, &base, None);
            let token = reg
                .authenticate("org/versioned", "v2.3.1")
                .await
                .expect("authenticate");
            assert_eq!(token, "versioned_tok");
        }

        // ------------------------------------------------------------------
        // Streaming layer storage
        // ------------------------------------------------------------------

        #[tokio::test]
        async fn pull_image_rejects_digest_mismatch() {
            let dir = TempDir::new().unwrap();
            let store = make_store(&dir);
            let server = MockServer::start().await;
            let base = format!("{}/v2", server.uri());
            let (layer_bytes, _real_digest) = make_test_layer();
            let size = layer_bytes.len();

            // Use a fake digest that does NOT match the layer bytes.
            let fake_digest = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
            let manifest = serde_json::to_vec(&manifest_json(fake_digest, size)).unwrap();
            mount_auth_with_manifest(&server, "org/bad", "latest", "bad_tok", manifest).await;

            Mock::given(method("GET"))
                .and(path_regex(r"^/v2/org/bad/blobs/sha256:.*"))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(layer_bytes))
                .expect(1)
                .mount(&server)
                .await;

            let reg = GhcrRegistry::for_test(Arc::clone(&store), &base, None);
            let image_ref =
                crate::image::reference::ImageRef::parse("ghcr.io/org/bad:latest").expect("ref");
            let err = reg.pull_image(&image_ref).await;
            assert!(err.is_err(), "expected digest mismatch error");
            let err = err.unwrap_err();
            let chain = format!("{err:#}");
            assert!(
                chain.contains("mismatch") || chain.contains("digest"),
                "expected digest error, got: {chain}"
            );
        }

        #[tokio::test]
        async fn pull_image_cleans_tmp_on_digest_mismatch() {
            let dir = TempDir::new().unwrap();
            let store = make_store(&dir);
            let server = MockServer::start().await;
            let base = format!("{}/v2", server.uri());
            let (layer_bytes, _real_digest) = make_test_layer();
            let size = layer_bytes.len();

            let fake_digest = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
            let manifest = serde_json::to_vec(&manifest_json(fake_digest, size)).unwrap();
            mount_auth_with_manifest(&server, "org/dirty", "latest", "dirty_tok", manifest).await;

            Mock::given(method("GET"))
                .and(path_regex(r"^/v2/org/dirty/blobs/sha256:.*"))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(layer_bytes))
                .expect(1)
                .mount(&server)
                .await;

            let reg = GhcrRegistry::for_test(Arc::clone(&store), &base, None);
            let image_ref =
                crate::image::reference::ImageRef::parse("ghcr.io/org/dirty:latest").expect("ref");
            let _ = reg.pull_image(&image_ref).await;

            // No layer directory or tmp directory should remain after a failed pull.
            let layers_dir = dir.path().join("images").join("ghcr.io_org_dirty").join("latest").join("layers");
            if layers_dir.exists() {
                let entries: Vec<_> = std::fs::read_dir(&layers_dir)
                    .expect("read layers dir")
                    .collect();
                assert!(
                    entries.is_empty(),
                    "layers dir should be empty after digest mismatch, found: {entries:?}"
                );
            }
        }

        #[tokio::test]
        async fn pull_image_stores_layer_on_disk() {
            let dir = TempDir::new().unwrap();
            let store = make_store(&dir);
            let server = MockServer::start().await;
            let base = format!("{}/v2", server.uri());
            let (layer_bytes, digest) = make_test_layer();
            let size = layer_bytes.len();

            let manifest = serde_json::to_vec(&manifest_json(&digest, size)).unwrap();
            mount_auth_with_manifest(&server, "org/streamed", "latest", "stream_tok", manifest)
                .await;

            Mock::given(method("GET"))
                .and(path_regex(r"^/v2/org/streamed/blobs/sha256:.*"))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(layer_bytes))
                .expect(1)
                .mount(&server)
                .await;

            let reg = GhcrRegistry::for_test(Arc::clone(&store), &base, None);
            let image_ref = crate::image::reference::ImageRef::parse("ghcr.io/org/streamed:latest")
                .expect("parse ref");
            reg.pull_image(&image_ref).await.expect("pull_image");

            // Layer directory must exist after pull.
            let layers = reg
                .get_image_layers("ghcr.io/org/streamed", "latest")
                .expect("get_image_layers");
            assert!(!layers.is_empty(), "expected at least one layer dir");
            assert!(layers[0].exists(), "layer dir should exist on disk");
        }
    }
}
