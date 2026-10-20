//! Network adapters implementing [`crate::domain::NetworkProvider`].
//!
//! Each adapter corresponds to a [`crate::domain::NetworkMode`] variant:
//!
//! - [`NoopNetwork`] — `NetworkMode::None`; isolated namespace, no connectivity
//! - [`HostNetwork`] — `NetworkMode::Host`; shares the host network namespace

#[cfg(target_os = "linux")]
pub mod bridge;
pub mod host;
pub mod none;

#[cfg(target_os = "linux")]
pub use bridge::BridgeNetwork;
pub use host::HostNetwork;
pub use none::NoopNetwork;
