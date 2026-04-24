//! Cross-platform adapter implementations.
//!
//! This module contains the adapters that live in `minibox-core`:
//! - [`DockerHubRegistry`]: Docker Hub implementation of [`crate::domain::ImageRegistry`]
//! - `mocks`: Mock test doubles for all four domain traits (behind `test-utils` feature)
//! - `test_fixtures`: Shared test setup helpers (behind `test-utils` feature)

mod noop_exec;
mod registry;
mod registry_router;

pub use noop_exec::NoopExecRuntime;
pub use registry_router::HostnameRegistryRouter;

/// Mock adapters for all domain traits, for use in tests.
///
/// Enabled via the `test-utils` Cargo feature so that other crates can depend
/// on these mocks in their `[dev-dependencies]` without `cfg(test)` restrictions.
#[cfg(any(test, feature = "test-utils"))]
pub mod mocks;

/// Shared test fixture builders.
///
/// Enabled via the `test-utils` Cargo feature so that other crates can use
/// these fixtures in their own integration tests.
#[cfg(any(test, feature = "test-utils"))]
pub mod test_fixtures;

/// Conformance test infrastructure for commit/build/push adapter backends.
///
/// Provides [`conformance::BackendDescriptor`] and shared fixture helpers.
/// Enabled via the `test-utils` Cargo feature.
#[cfg(any(test, feature = "test-utils"))]
pub mod conformance;

pub use registry::DockerHubRegistry;
