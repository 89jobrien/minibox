//! miniboxd library — re-exports from minibox::daemon for backward compatibility.
//!
//! These re-exports exist so that integration tests importing
//! `miniboxd::handler`, `miniboxd::state`, or `miniboxd::server` continue
//! to compile without changes.

#[doc(hidden)]
pub use minibox::daemon::handler;
#[doc(hidden)]
pub use minibox::daemon::server;
#[doc(hidden)]
pub use minibox::daemon::state;
