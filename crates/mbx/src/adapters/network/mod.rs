//! Network adapters implementing [`NetworkProvider`].
//!
//! Each adapter corresponds to a [`NetworkMode`] variant:
//!
//! - [`NoopNetwork`] — `NetworkMode::None`; isolated namespace, no connectivity
//! - [`HostNetwork`] — `NetworkMode::Host`; shares the host network namespace

pub mod host;
pub mod none;

pub use host::HostNetwork;
pub use none::NoopNetwork;
