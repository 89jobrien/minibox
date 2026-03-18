//! miniboxd library — re-exports from daemonbox for backward compatibility.
//!
//! These re-exports exist so that integration tests importing
//! `miniboxd::handler`, `miniboxd::state`, or `miniboxd::server` continue
//! to compile without changes after the move to `daemonbox`.

#[doc(hidden)]
pub use daemonbox::handler;
#[doc(hidden)]
pub use daemonbox::server;
#[doc(hidden)]
pub use daemonbox::state;
