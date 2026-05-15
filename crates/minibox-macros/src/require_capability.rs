//! `require_capability!` -- skip a test when a required host capability is absent.
//!
//! If `$caps.$field` is `false`, prints a `SKIPPED: $reason` message to stderr
//! and returns from the calling function early. Intended for integration tests
//! that probe host state before executing privileged setup.
//!
//! # Examples
//!
//! ```rust,ignore
//! minibox_macros::require_capability!(caps, is_root, "requires root");
//! ```

#[macro_export]
macro_rules! require_capability {
    ($caps:expr, $field:ident, $reason:expr) => {
        if !$caps.$field {
            ::std::eprintln!("SKIPPED: {}", $reason);
            return;
        }
    };
}
