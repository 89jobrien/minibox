use std::path::PathBuf;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// MinimalStoredImageFixture
// ---------------------------------------------------------------------------

/// A temporary directory tree that mimics the on-disk layout of a stored OCI
/// image with one empty layer.
///
/// Layout:
/// ```text
/// <tmp>/
///   images/
///     <name>/
///       <digest>/    ← layer dir (empty — no filesystem entries)
///   manifests/
///     <name>.json   ← placeholder file (content not validated by fixture)
/// ```
///
/// Useful as the source image for commit and build conformance tests that need
/// a pre-existing "base" image in the store.
pub struct MinimalStoredImageFixture {
    /// Root temp dir (kept alive for [`Drop`]).
    pub dir: TempDir,
    /// `<tmp>/images/` — root of the image store.
    pub images_dir: PathBuf,
    /// `<tmp>/images/<name>/<digest>/` — the single empty layer directory.
    pub layer_dir: PathBuf,
    /// OCI digest string (sha256:000…0) used for the placeholder layer.
    pub layer_digest: String,
    /// Image name used to construct `layer_dir`.
    pub image_name: String,
}

impl MinimalStoredImageFixture {
    /// Create a minimal stored image fixture under a fresh temporary directory.
    ///
    /// `image_name` is used verbatim as the subdirectory name under `images/`.
    /// If `None`, the name `"conformance-base"` is used.
    pub fn new(image_name: Option<&str>) -> std::io::Result<Self> {
        let name = image_name.unwrap_or("conformance-base").to_string();
        // Use a deterministic but clearly fake digest so tests that check digest
        // values get a predictable string without requiring SHA computation.
        let digest =
            "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string();
        let stripped = digest.strip_prefix("sha256:").unwrap_or(&digest);

        let dir = TempDir::new()?;
        let images_dir = dir.path().join("images");
        let layer_dir = images_dir.join(&name).join(stripped);
        std::fs::create_dir_all(&layer_dir)?;

        let manifests_dir = dir.path().join("manifests");
        std::fs::create_dir_all(&manifests_dir)?;
        // Placeholder manifest — conformance tests must not parse this file;
        // it exists only so the directory tree looks complete.
        std::fs::write(
            manifests_dir.join(format!("{name}.json")),
            b"{\"placeholder\":true}\n",
        )?;

        Ok(Self {
            dir,
            images_dir,
            layer_dir,
            layer_digest: digest,
            image_name: name,
        })
    }
}
