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
