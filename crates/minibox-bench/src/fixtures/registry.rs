//! Wiremock-backed OCI registry fixture for pull benchmarks.
//!
//! Mirrors the endpoint shapes exercised by the in-crate wiremock harness in
//! `minibox-core/src/image/registry.rs` tests: a token endpoint, a manifest
//! endpoint returning a single OCI image manifest, and one blob endpoint per
//! layer.

// Bench fixture code: panicking on a broken fixture is the correct behaviour.
#![allow(clippy::expect_used)]

use anyhow::{Context, Result};
use minibox_core::RegistryClient;
use minibox_core::image::manifest::TargetPlatform;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::fixtures::layer::sha256_digest;

const MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
const LAYER_MEDIA_TYPE: &str = "application/vnd.oci.image.layer.v1.tar+gzip";
const CONFIG_MEDIA_TYPE: &str = "application/vnd.oci.image.config.v1+json";

/// Wiremock-backed OCI registry serving one image.
pub struct BenchRegistry {
    server: MockServer,
}

impl BenchRegistry {
    /// Serve `image:tag` composed of the given gzipped tar layers.
    ///
    /// Registers a token endpoint, a manifest endpoint whose layer digests
    /// and sizes are derived from `layers`, and one blob endpoint per layer.
    ///
    /// # Errors
    ///
    /// Currently infallible in practice; returns `Result` so the harness API
    /// can surface future setup failures without a signature change.
    pub async fn serve(image: &str, tag: &str, layers: Vec<Vec<u8>>) -> Result<Self> {
        let server = MockServer::start().await;

        // Token endpoint: RegistryClient::authenticate GETs the auth URL and
        // expects a JSON body with a `token` field.
        Mock::given(method("GET"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(r#"{"token":"benchtoken"}"#, "application/json"),
            )
            .mount(&server)
            .await;

        // Manifest endpoint: single OCI image manifest listing every layer
        // with its real digest and size. The config blob is never fetched by
        // pull_image, so a placeholder digest is sufficient.
        let layer_entries: Vec<String> = layers
            .iter()
            .map(|bytes| {
                format!(
                    r#"{{"mediaType":"{LAYER_MEDIA_TYPE}","size":{},"digest":"{}"}}"#,
                    bytes.len(),
                    sha256_digest(bytes)
                )
            })
            .collect();
        let manifest = format!(
            r#"{{"schemaVersion":2,"mediaType":"{MANIFEST_MEDIA_TYPE}","config":{{"mediaType":"{CONFIG_MEDIA_TYPE}","size":10,"digest":"sha256:config"}},"layers":[{}]}}"#,
            layer_entries.join(",")
        );
        Mock::given(method("GET"))
            .and(path(format!("/v2/{image}/manifests/{tag}")))
            .respond_with(ResponseTemplate::new(200).set_body_raw(manifest, MANIFEST_MEDIA_TYPE))
            .mount(&server)
            .await;

        // Blob endpoints: one per layer, keyed by digest. path_regex because
        // reqwest may percent-encode ':' in "sha256:..." as "sha256%3A...".
        for bytes in layers {
            let digest = sha256_digest(&bytes);
            let hex = digest
                .strip_prefix("sha256:")
                .expect("sha256_digest always emits a sha256: prefix");
            Mock::given(method("GET"))
                .and(path_regex(format!(r"/blobs/sha256(:|%3A){hex}$")))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/octet-stream")
                        .set_body_bytes(bytes),
                )
                .mount(&server)
                .await;
        }

        Ok(Self { server })
    }

    /// `RegistryClient` pointed at the mock server, pinned to linux/amd64.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP client cannot be constructed.
    pub fn client(&self) -> Result<RegistryClient> {
        let uri = self.server.uri();
        Ok(
            RegistryClient::for_test(&format!("{uri}/token"), &format!("{uri}/v2"))
                .context("construct bench RegistryClient")?
                .with_pinned_platform(TargetPlatform::linux_amd64()),
        )
    }

    /// Base URI of the mock server.
    #[must_use]
    pub fn uri(&self) -> String {
        self.server.uri()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::layer::{LayerSpec, build_layer_tar_gz};
    use minibox_core::ImageStore;

    #[tokio::test]
    async fn pull_image_through_bench_registry_succeeds() {
        let layer = build_layer_tar_gz(&LayerSpec {
            file_count: 3,
            file_size_bytes: 1024,
            dir_depth: 2,
        });
        let registry = BenchRegistry::serve("bench/img", "latest", vec![layer])
            .await
            .expect("serve bench registry");

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let store = ImageStore::new(tmp.path().join("images")).expect("image store");

        registry
            .client()
            .expect("bench client")
            .pull_image("bench/img", "latest", &store)
            .await
            .expect("pull_image against bench registry");

        assert!(store.has_image("bench/img", "latest"));
        let layers = store
            .get_image_layers("bench/img", "latest")
            .expect("stored layers");
        assert_eq!(layers.len(), 1, "one layer expected");
        let extracted = std::fs::read_dir(&layers[0])
            .expect("extracted layer dir readable")
            .count();
        assert!(extracted > 0, "extracted layer dir should be non-empty");
    }
}
