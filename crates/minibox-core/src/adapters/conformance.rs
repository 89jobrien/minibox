//! Conformance test infrastructure for commit/build/push adapter backends.
//!
//! This module is gated behind the `test-utils` feature so downstream crates
//! can depend on it in `[dev-dependencies]` without pulling test code into
//! production builds.
//!
//! # What lives here
//!
//! - [`BackendDescriptor`] — describes a concrete backend under test with its
//!   capability flags and zero-argument constructor hooks for each adapter.
//! - Fixture helpers — minimal, self-contained helpers that create the on-disk
//!   state a conformance test needs (stored image dirs, writable upper dirs,
//!   build contexts, local push target references).
//!
//! # Usage
//!
//! ```rust,ignore
//! use minibox_core::adapters::conformance::{
//!     BackendDescriptor, BuildContextFixture, MinimalStoredImageFixture,
//!     WritableUpperDirFixture, LocalPushTargetFixture,
//! };
//! use minibox_core::domain::BackendCapability;
//!
//! fn linux_native_backend(fixture: &MinimalStoredImageFixture) -> BackendDescriptor {
//!     BackendDescriptor::new("linux-native")
//!         .with_capability(BackendCapability::Commit)
//!         .with_capability(BackendCapability::BuildFromContext)
//!         .with_capability(BackendCapability::PushToRegistry)
//! }
//!
//! #[tokio::test]
//! async fn conformance_commit_roundtrip() {
//!     let img = MinimalStoredImageFixture::new().unwrap();
//!     let upper = WritableUpperDirFixture::new().unwrap();
//!     let backend = linux_native_backend(&img);
//!
//!     if !backend.capabilities.supports(BackendCapability::Commit) {
//!         return; // skip — backend does not support commit
//!     }
//!     // drive backend.make_committer() …
//! }
//! ```

use crate::domain::{
    BackendCapability, BackendCapabilitySet, DynContainerCommitter, DynImageBuilder, DynImagePusher,
};
use std::path::PathBuf;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// BackendDescriptor
// ---------------------------------------------------------------------------

/// Describes a concrete backend under conformance test.
///
/// Each field is optional: `None` means the backend does not provide that
/// adapter and any conformance test for that capability must be skipped (check
/// `capabilities.supports(cap)` first).
///
/// # Constructor hooks
///
/// The `make_*` fields hold `Box<dyn Fn() -> …>` rather than the adapters
/// themselves so that:
/// - Construction is deferred until the test actually needs the adapter.
/// - Each test invocation gets a fresh adapter instance (no shared mutable
///   state leaking between test cases).
///
/// The closures take no arguments — all required context (image store paths,
/// daemon state handles, etc.) must be captured from the surrounding fixture.
pub struct BackendDescriptor {
    /// Human-readable identifier used in test failure messages.
    pub name: &'static str,

    /// The set of capabilities this backend declares.
    pub capabilities: BackendCapabilitySet,

    /// Factory for a fresh [`DynContainerCommitter`], or `None` when
    /// `BackendCapability::Commit` is absent.
    pub make_committer: Option<Box<dyn Fn() -> DynContainerCommitter + Send + Sync>>,

    /// Factory for a fresh [`DynImageBuilder`], or `None` when
    /// `BackendCapability::BuildFromContext` is absent.
    pub make_builder: Option<Box<dyn Fn() -> DynImageBuilder + Send + Sync>>,

    /// Factory for a fresh [`DynImagePusher`], or `None` when
    /// `BackendCapability::PushToRegistry` is absent.
    pub make_pusher: Option<Box<dyn Fn() -> DynImagePusher + Send + Sync>>,
}

impl BackendDescriptor {
    /// Create a descriptor with the given name and no capabilities.
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            capabilities: BackendCapabilitySet::new(),
            make_committer: None,
            make_builder: None,
            make_pusher: None,
        }
    }

    /// Declare that this backend supports `cap`.
    pub fn with_capability(mut self, cap: BackendCapability) -> Self {
        self.capabilities = self.capabilities.with(cap);
        self
    }

    /// Attach a committer factory (implies `BackendCapability::Commit`).
    pub fn with_committer<F>(mut self, f: F) -> Self
    where
        F: Fn() -> DynContainerCommitter + Send + Sync + 'static,
    {
        self.capabilities = self.capabilities.with(BackendCapability::Commit);
        self.make_committer = Some(Box::new(f));
        self
    }

    /// Attach a builder factory (implies `BackendCapability::BuildFromContext`).
    pub fn with_builder<F>(mut self, f: F) -> Self
    where
        F: Fn() -> DynImageBuilder + Send + Sync + 'static,
    {
        self.capabilities = self.capabilities.with(BackendCapability::BuildFromContext);
        self.make_builder = Some(Box::new(f));
        self
    }

    /// Attach a pusher factory (implies `BackendCapability::PushToRegistry`).
    pub fn with_pusher<F>(mut self, f: F) -> Self
    where
        F: Fn() -> DynImagePusher + Send + Sync + 'static,
    {
        self.capabilities = self.capabilities.with(BackendCapability::PushToRegistry);
        self.make_pusher = Some(Box::new(f));
        self
    }
}

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
        let digest = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
            .to_string();
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
/// [`ContainerCommitter`]: crate::domain::ContainerCommitter
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
/// [`ImageBuilder`]: crate::domain::ImageBuilder
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

        std::fs::write(
            &dockerfile,
            b"FROM scratch\nCOPY hello.txt /hello.txt\n",
        )?;
        std::fs::write(context_dir.join("hello.txt"), b"conformance\n")?;

        Ok(Self {
            dir,
            context_dir,
            dockerfile,
        })
    }
}

// ---------------------------------------------------------------------------
// LocalPushTargetFixture
// ---------------------------------------------------------------------------

/// A locally-resolvable push target reference for use in push conformance
/// tests.
///
/// Real push conformance tests require a running OCI registry. This fixture
/// provides a reference string pointing to `localhost:5000` (the conventional
/// local registry port) and documents the expectation that the test runner
/// must ensure a registry is available at that address.
///
/// The fixture does **not** start a registry — it only provides a consistent,
/// predictable reference string and associated metadata so all conformance
/// tests use the same target convention.
pub struct LocalPushTargetFixture {
    /// The full image reference string, e.g.
    /// `"localhost:5000/conformance/push-test:latest"`.
    pub image_ref: String,
    /// The registry host portion, e.g. `"localhost:5000"`.
    pub registry_host: String,
    /// The repository path, e.g. `"conformance/push-test"`.
    pub repository: String,
    /// The tag, always `"latest"` for conformance tests.
    pub tag: String,
}

impl LocalPushTargetFixture {
    /// Construct a local push target for `repository` on `localhost:5000`.
    ///
    /// `repository` should be a path like `"conformance/push-test"`.
    pub fn new(repository: &str) -> Self {
        let registry_host = "localhost:5000".to_string();
        let tag = "latest".to_string();
        let image_ref = format!("{registry_host}/{repository}:{tag}");
        Self {
            image_ref,
            registry_host,
            repository: repository.to_string(),
            tag,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::BackendCapability;

    // --- BackendDescriptor ---

    #[test]
    fn descriptor_starts_with_no_capabilities() {
        let d = BackendDescriptor::new("test-backend");
        assert!(!d.capabilities.supports(BackendCapability::Commit));
        assert!(!d.capabilities.supports(BackendCapability::BuildFromContext));
        assert!(!d.capabilities.supports(BackendCapability::PushToRegistry));
    }

    #[test]
    fn descriptor_with_capability_adds_flag() {
        let d = BackendDescriptor::new("test-backend")
            .with_capability(BackendCapability::PushToRegistry);
        assert!(d.capabilities.supports(BackendCapability::PushToRegistry));
        assert!(!d.capabilities.supports(BackendCapability::Commit));
    }

    // --- MinimalStoredImageFixture ---

    #[test]
    fn minimal_stored_image_fixture_creates_layer_dir() {
        let f = MinimalStoredImageFixture::new(None).expect("fixture creation");
        assert!(f.layer_dir.exists(), "layer dir must exist");
        assert!(f.layer_dir.is_dir(), "layer dir must be a directory");
        assert!(f.images_dir.exists());
    }

    #[test]
    fn minimal_stored_image_fixture_custom_name() {
        let f = MinimalStoredImageFixture::new(Some("my-image")).expect("fixture creation");
        assert_eq!(f.image_name, "my-image");
        assert!(f.layer_dir.exists());
    }

    #[test]
    fn minimal_stored_image_fixture_manifest_placeholder_exists() {
        let f = MinimalStoredImageFixture::new(None).expect("fixture creation");
        let manifest = f
            .dir
            .path()
            .join("manifests")
            .join(format!("{}.json", f.image_name));
        assert!(manifest.exists(), "placeholder manifest must exist");
    }

    // --- WritableUpperDirFixture ---

    #[test]
    fn writable_upper_dir_fixture_creates_sentinel() {
        let f = WritableUpperDirFixture::new().expect("fixture creation");
        assert!(f.upper_dir.exists());
        assert!(f.work_dir.exists());
        let sentinel = f.upper_dir.join(f.sentinel_filename);
        assert!(sentinel.exists(), "sentinel file must exist in upperdir");
        let content = std::fs::read(&sentinel).unwrap();
        assert_eq!(content, b"1");
    }

    // --- BuildContextFixture ---

    #[test]
    fn build_context_fixture_creates_dockerfile() {
        let f = BuildContextFixture::new().expect("fixture creation");
        assert!(f.dockerfile.exists(), "Dockerfile must exist");
        let content = std::fs::read_to_string(&f.dockerfile).unwrap();
        assert!(content.contains("FROM scratch"));
        let hello = f.context_dir.join("hello.txt");
        assert!(hello.exists(), "hello.txt must exist in context");
    }

    // --- LocalPushTargetFixture ---

    #[test]
    fn local_push_target_fixture_formats_ref_correctly() {
        let f = LocalPushTargetFixture::new("conformance/push-test");
        assert_eq!(
            f.image_ref,
            "localhost:5000/conformance/push-test:latest"
        );
        assert_eq!(f.registry_host, "localhost:5000");
        assert_eq!(f.tag, "latest");
    }
}
