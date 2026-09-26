//! Adapters implementing the crate's ports against the real process and
//! filesystem.
//!
//! The domain modules — [`crate::version`], [`crate::plugin`],
//! [`crate::rollout`] — are pure functions. Everything that reads the
//! process environment, spawns a child, or stats a file lives here, which
//! is what lets the protocol and rollout logic be tested in-memory.

pub mod env;
pub mod namespace;
pub mod probe;

pub use env::ProcessEnvironment;
pub use namespace::UnimplementedNamespaceAttachment;
pub use probe::ExecPluginProbe;
