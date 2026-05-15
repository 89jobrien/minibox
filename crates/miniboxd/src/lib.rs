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

#[cfg(unix)]
#[doc(hidden)]
pub use minibox::daemon::handler;
#[cfg(unix)]
#[doc(hidden)]
pub use minibox::daemon::server;
#[cfg(unix)]
#[doc(hidden)]
pub use minibox::daemon::state;
