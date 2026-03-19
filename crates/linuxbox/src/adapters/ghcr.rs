//! GitHub Container Registry (ghcr.io) adapter implementing the ImageRegistry trait.
//!
//! Authenticates via `WWW-Authenticate` Bearer challenge. Pass a personal access
//! token (PAT) with `read:packages` scope as `GHCR_TOKEN` to access private images;
//! public images work without a token.

use crate::as_any;
use crate::domain::{ImageMetadata, ImageRegistry, LayerInfo};
use crate::image::ImageStore;
use crate::image::manifest::ManifestResponse;
use anyhow::{Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
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
        Ok(Self { store, token, http })
    }

    // -----------------------------------------------------------------------
    // Authentication
    // -----------------------------------------------------------------------

    /// Obtain a Bearer token for `repo` via the `WWW-Authenticate` challenge.
    ///
    /// Returns an empty string for unauthenticated public images.
    async fn authenticate(&self, repo: &str) -> Result<String> {
        // Probe the registry — it will respond with 401 + WWW-Authenticate.
        let url = format!("{GHCR_BASE}/{repo}/manifests/latest");
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
        let url = format!("{GHCR_BASE}/{repo}/manifests/{tag_or_digest}");
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
                let desc = list.find_linux_amd64().ok_or_else(|| {
                    anyhow::anyhow!(
                        "ghcr: no linux/amd64 manifest in list for {repo}:{tag_or_digest}",
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

    /// Download a single blob by `digest` and return its raw bytes.
    async fn pull_layer(&self, repo: &str, digest: &str, token: &str) -> Result<Bytes> {
        let url = format!("{GHCR_BASE}/{repo}/blobs/{digest}");
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

        // SECURITY: Check Content-Length before downloading.
        if let Some(cl) = resp.headers().get("content-length")
            && let Ok(s) = cl.to_str()
            && let Ok(n) = s.parse::<u64>()
            && n > MAX_LAYER_SIZE
        {
            anyhow::bail!("ghcr: layer too large: {n} bytes (max {MAX_LAYER_SIZE})");
        }

        let bytes = resp
            .bytes()
            .await
            .with_context(|| format!("ghcr: reading blob body for {digest}"))?;

        anyhow::ensure!(
            bytes.len() as u64 <= MAX_LAYER_SIZE,
            "ghcr: layer {digest} exceeded size limit",
        );

        Ok(bytes)
    }
}

as_any!(GhcrRegistry);

// ---------------------------------------------------------------------------
// ImageRegistry trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ImageRegistry for GhcrRegistry {
    async fn has_image(&self, name: &str, tag: &str) -> bool {
        // GHCR images stored with "ghcr.io/" prefix to avoid collisions.
        let store_key = format!("ghcr.io/{name}");
        self.store.has_image(&store_key, tag)
    }

    async fn pull_image(&self, name: &str, tag: &str) -> Result<ImageMetadata> {
        let store_key = format!("ghcr.io/{name}");
        info!("ghcr: pulling {store_key}:{tag}");

        let token = self
            .authenticate(name)
            .await
            .with_context(|| format!("ghcr: authenticate for {name}"))?;

        let manifest = self
            .get_manifest(name, tag, &token)
            .await
            .with_context(|| format!("ghcr: get manifest for {name}:{tag}"))?;

        let mut layer_infos = Vec::new();
        for layer in &manifest.layers {
            let digest = &layer.digest;
            let data = self
                .pull_layer(name, digest, &token)
                .await
                .with_context(|| format!("ghcr: pull layer {digest}"))?;

            self.store
                .store_layer(&store_key, tag, digest, std::io::Cursor::new(data))
                .with_context(|| format!("ghcr: store layer {digest}"))?;

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
        let store_key = format!("ghcr.io/{name}");
        self.store.get_image_layers(&store_key, tag)
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

    #[test]
    fn ghcr_registry_constructs() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(dir.path().join("images")).unwrap());
        let reg = GhcrRegistry::new(store);
        assert!(reg.is_ok());
    }
}
