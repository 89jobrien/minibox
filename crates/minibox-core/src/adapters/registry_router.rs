//! [`HostnameRegistryRouter`] — routes image references to registry adapters by hostname.

use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::{DynImageRegistry, ImageRegistry, RegistryRouter};
use crate::image::reference::ImageRef;

/// Routes image references to the appropriate [`ImageRegistry`] adapter based on
/// the image's registry hostname.
///
/// Lookup is case-insensitive (hostname is lowercased before matching).  If no
/// override matches, the `default` registry is used.
///
/// # Example
///
/// ```rust,ignore
/// use minibox_core::adapters::HostnameRegistryRouter;
///
/// let router = HostnameRegistryRouter::new(
///     docker_hub,
///     [("ghcr.io", ghcr)],
/// );
/// let registry = router.route(&image_ref);
/// ```
pub struct HostnameRegistryRouter {
    default: DynImageRegistry,
    overrides: HashMap<String, DynImageRegistry>,
}

impl HostnameRegistryRouter {
    /// Construct a router with `default` as the fallback registry and
    /// `overrides` mapping lowercase hostnames to specific adapters.
    ///
    /// Keys in `overrides` are stored as lowercase; callers need not pre-lowercase them.
    pub fn new(
        default: DynImageRegistry,
        overrides: impl IntoIterator<Item = (impl Into<String>, DynImageRegistry)>,
    ) -> Self {
        let overrides = overrides
            .into_iter()
            .map(|(k, v)| (k.into().to_lowercase(), v))
            .collect();
        Self { default, overrides }
    }
}

impl RegistryRouter for HostnameRegistryRouter {
    fn route(&self, image_ref: &ImageRef) -> &dyn ImageRegistry {
        let hostname = image_ref.registry.to_lowercase();
        self.overrides
            .get(&hostname)
            .map(Arc::as_ref)
            .unwrap_or_else(|| self.default.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::reference::ImageRef;

    struct StubRegistry {
        label: &'static str,
    }

    impl crate::domain::AsAny for StubRegistry {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[async_trait::async_trait]
    impl ImageRegistry for StubRegistry {
        async fn has_image(&self, _name: &str, _tag: &str) -> bool {
            false
        }
        async fn pull_image(
            &self,
            _image_ref: &ImageRef,
        ) -> anyhow::Result<crate::domain::ImageMetadata> {
            unimplemented!()
        }
        fn get_image_layers(
            &self,
            _name: &str,
            _tag: &str,
        ) -> anyhow::Result<Vec<std::path::PathBuf>> {
            unimplemented!()
        }
    }

    fn make_router() -> (HostnameRegistryRouter, *const (), *const ()) {
        let docker: DynImageRegistry = Arc::new(StubRegistry { label: "docker" });
        let ghcr: DynImageRegistry = Arc::new(StubRegistry { label: "ghcr" });

        let docker_ptr = Arc::as_ptr(&docker) as *const ();
        let ghcr_ptr = Arc::as_ptr(&ghcr) as *const ();

        let router = HostnameRegistryRouter::new(docker, [("ghcr.io", ghcr)]);
        (router, docker_ptr, ghcr_ptr)
    }

    #[test]
    fn routes_ghcr() {
        let (router, _, ghcr_ptr) = make_router();
        let image_ref = ImageRef::parse("ghcr.io/org/minibox-rust-ci:stable").unwrap();
        let selected = router.route(&image_ref) as *const dyn ImageRegistry as *const ();
        assert_eq!(selected, ghcr_ptr);
    }

    #[test]
    fn routes_ghcr_case_insensitive() {
        let (router, _, ghcr_ptr) = make_router();
        let image_ref = ImageRef::parse("GHCR.IO/org/image:tag").unwrap();
        let selected = router.route(&image_ref) as *const dyn ImageRegistry as *const ();
        assert_eq!(selected, ghcr_ptr);
    }

    #[test]
    fn routes_default_for_docker_hub() {
        let (router, docker_ptr, _) = make_router();
        let image_ref = ImageRef::parse("alpine").unwrap();
        let selected = router.route(&image_ref) as *const dyn ImageRegistry as *const ();
        assert_eq!(selected, docker_ptr);
    }

    #[test]
    fn routes_default_for_unknown_hostname() {
        let (router, docker_ptr, _) = make_router();
        let image_ref = ImageRef::parse("quay.io/prometheus/alertmanager:latest").unwrap();
        let selected = router.route(&image_ref) as *const dyn ImageRegistry as *const ();
        assert_eq!(selected, docker_ptr);
    }
}
