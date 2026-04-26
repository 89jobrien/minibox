use std::path::PathBuf;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// WritableUpperDirFixture
// ---------------------------------------------------------------------------

/// A temporary directory pair simulating an overlay FS upper + work dir.
///
/// Conformance tests for [`ContainerCommitter`] need a writable upperdir that
/// contains a diff to snapshot. This fixture seeds the upperdir with a single
/// known file so tests can assert the commit result includes it.
///
/// Layout:
/// ```text
/// <tmp>/
///   upper/
///     conformance-sentinel   ← 1-byte sentinel file
///   work/                    ← empty work dir (required by overlayfs)
/// ```
///
/// [`ContainerCommitter`]: minibox_core::domain::ContainerCommitter
pub struct WritableUpperDirFixture {
    /// Root temp dir (kept alive for [`Drop`]).
    pub dir: TempDir,
    /// `<tmp>/upper/` — the overlay FS upperdir path.
    pub upper_dir: PathBuf,
    /// `<tmp>/work/` — the overlay FS workdir path.
    pub work_dir: PathBuf,
    /// Name of the sentinel file placed in `upper_dir`.
    pub sentinel_filename: &'static str,
}

impl WritableUpperDirFixture {
    /// Create the fixture, writing a sentinel file into the upperdir.
    pub fn new() -> std::io::Result<Self> {
        let dir = TempDir::new()?;
        let upper_dir = dir.path().join("upper");
        let work_dir = dir.path().join("work");
        std::fs::create_dir_all(&upper_dir)?;
        std::fs::create_dir_all(&work_dir)?;

        let sentinel_filename = "conformance-sentinel";
        std::fs::write(upper_dir.join(sentinel_filename), b"1")?;

        Ok(Self {
            dir,
            upper_dir,
            work_dir,
            sentinel_filename,
        })
    }
}
