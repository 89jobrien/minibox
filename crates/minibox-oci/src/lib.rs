//! # minibox-oci
//!
//! Standalone OCI image-pulling library.
//!
//! Provides a high-level [`pull`] function and lower-level types for working
//! with OCI registries, image stores, manifests, and tar layers.
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use minibox_oci::pull;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     pull("alpine:latest", "/var/lib/minibox/images", |progress| {
//!         println!("{progress}");
//!     })
//!     .await
//! }
//! ```
//!
//! ## Module overview
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`image`] | OCI image handling: [`image::ImageStore`], manifest types, layer extraction. |
//! | [`image::registry`] | Docker Hub v2 registry client with anonymous token auth. |
//! | [`image::reference`] | Parse `[registry/]namespace/name[:tag]` image references. |
//! | [`image::manifest`] | OCI manifest and manifest list types. |
//! | [`image::layer`] | Tar layer extraction with path-traversal protection. |
//! | [`image::gc`] | Image garbage collection. |
//! | [`image::lease`] | GC-protection leases. |
//! | [`image::dockerfile`] | Basic Dockerfile parser. |
//! | [`error`] | [`error::ImageError`] and [`error::RegistryError`] types. |

pub mod error;
pub mod image;

// Top-level convenience re-exports.
pub use error::{ImageError, RegistryError};
pub use image::ImageStore;
pub use image::reference::{ImageRef, ImageRefError};
pub use image::registry::RegistryClient;

/// Pull an OCI image into a local store.
///
/// Parses `image_ref` (e.g. `"alpine:latest"`, `"ghcr.io/org/name:tag"`),
/// authenticates against the appropriate registry, downloads the manifest and
/// all layer blobs, and extracts them under `store_path`.
///
/// `progress` is called with a human-readable status string for each major
/// step (auth, manifest fetch, each layer).  Pass a no-op closure to silence
/// progress output.
///
/// # Examples
///
/// ```rust,no_run
/// # #[tokio::main]
/// # async fn main() -> anyhow::Result<()> {
/// minibox_oci::pull("alpine:latest", "/var/lib/minibox/images", |msg| {
///     eprintln!("{msg}");
/// })
/// .await?;
/// # Ok(())
/// # }
/// ```
pub async fn pull(
    image_ref: &str,
    store_path: impl Into<std::path::PathBuf>,
    mut progress: impl FnMut(&str),
) -> anyhow::Result<()> {
    use anyhow::Context as _;

    let image_ref = ImageRef::parse(image_ref)
        .map_err(|e| anyhow::anyhow!("invalid image reference {image_ref:?}: {e}"))?;

    let store = ImageStore::new(store_path).context("opening image store")?;
    let client = RegistryClient::new().context("creating registry client")?;

    let name = image_ref.cache_name();
    let tag = &image_ref.tag;

    if store.has_image(&name, tag) {
        progress(&format!("image {name}:{tag} already cached"));
        return Ok(());
    }

    progress(&format!("authenticating for {name}"));
    let token = client
        .authenticate(&image_ref.repository())
        .await
        .with_context(|| format!("authenticating for {name}"))?;

    progress(&format!("fetching manifest for {name}:{tag}"));
    let manifest = client
        .get_manifest(&image_ref.repository(), tag, &token)
        .await
        .with_context(|| format!("fetching manifest for {name}:{tag}"))?;

    let layer_count = manifest.layers.len();
    for (i, layer) in manifest.layers.iter().enumerate() {
        progress(&format!(
            "pulling layer {}/{}: {}",
            i + 1,
            layer_count,
            &layer.digest[..layer.digest.len().min(20)]
        ));
        let blob = client
            .pull_layer(&image_ref.repository(), &layer.digest, &token)
            .await
            .with_context(|| format!("fetching layer {}", layer.digest))?;
        store
            .store_layer(&name, tag, &layer.digest, std::io::Cursor::new(&blob[..]))
            .with_context(|| format!("storing layer {}", layer.digest))?;
    }

    store
        .store_manifest(&name, tag, &manifest)
        .context("storing manifest")?;

    progress(&format!("pulled {name}:{tag} ({layer_count} layers)"));
    Ok(())
}
