//! Mock [`RegistryRouter`] for conformance testing.
//!
//! Routes all image references to a single backing [`ImageRegistry`],
//! regardless of hostname. Useful for tests that need a `DynRegistryRouter`
//! but do not care about routing logic.

use minibox_core::domain::{ImageRegistry, RegistryRouter};
use minibox_core::image::reference::ImageRef;
use std::sync::Arc;

/// Mock router that always returns the same registry for any image reference.
pub struct MockRegistryRouter {
    registry: Arc<dyn ImageRegistry>,
}

impl MockRegistryRouter {
    /// Create a router backed by the given registry.
    pub fn new(registry: Arc<dyn ImageRegistry>) -> Self {
        Self { registry }
    }
}

impl RegistryRouter for MockRegistryRouter {
    fn route(&self, _image_ref: &ImageRef) -> &dyn ImageRegistry {
        self.registry.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::mocks::registry::MockRegistry;

    #[test]
    fn route_returns_backing_registry() {
        let reg = Arc::new(MockRegistry::new());
        let router = MockRegistryRouter::new(reg);
        // Verify route() returns without panic for an arbitrary image ref.
        let image_ref = ImageRef::parse("alpine:3.18").expect("valid ref");
        let _ = router.route(&image_ref);
    }
}
