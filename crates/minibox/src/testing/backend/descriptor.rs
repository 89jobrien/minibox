use minibox_core::domain::{
    BackendCapability, BackendCapabilitySet, DynContainerCommitter, DynImageBuilder, DynImagePusher,
};

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
