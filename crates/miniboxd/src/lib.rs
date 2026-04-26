//! miniboxd library — re-exports from minibox::daemon for backward compatibility.
//!
//! Also provides [`adapter_registry`] for centralized adapter suite discovery.
//!
//! These re-exports exist so that integration tests importing
//! `miniboxd::handler`, `miniboxd::state`, or `miniboxd::server` continue
//! to compile without changes.

pub mod adapter_registry;
#[cfg(unix)]
pub mod listener;

#[doc(hidden)]
pub use minibox::daemon::handler;
#[doc(hidden)]
pub use minibox::daemon::server;
#[doc(hidden)]
pub use minibox::daemon::state;
