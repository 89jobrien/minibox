use std::path::PathBuf;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// BuildContextFixture
// ---------------------------------------------------------------------------

/// A minimal build context directory with a one-instruction Dockerfile.
///
/// Conformance tests for [`ImageBuilder`] need a real context directory on
/// disk. This fixture provides the smallest valid Dockerfile that produces a
/// deterministic image layer.
///
/// Layout:
/// ```text
/// <tmp>/
///   Dockerfile    ← `FROM scratch`
///   hello.txt     ← copied into image layer
/// ```
///
/// [`ImageBuilder`]: minibox_core::domain::ImageBuilder
pub struct BuildContextFixture {
    /// Root temp dir (kept alive for [`Drop`]).
    pub dir: TempDir,
    /// Path to the context directory (== `dir.path()`).
    pub context_dir: PathBuf,
    /// Path to the Dockerfile within `context_dir`.
    pub dockerfile: PathBuf,
}

impl BuildContextFixture {
    /// Create a minimal build context with a `FROM scratch` Dockerfile.
    pub fn new() -> std::io::Result<Self> {
        let dir = TempDir::new()?;
        let context_dir = dir.path().to_path_buf();
        let dockerfile = context_dir.join("Dockerfile");

        std::fs::write(&dockerfile, b"FROM scratch\nCOPY hello.txt /hello.txt\n")?;
        std::fs::write(context_dir.join("hello.txt"), b"conformance\n")?;

        Ok(Self {
            dir,
            context_dir,
            dockerfile,
        })
    }
}
