//! `test_run!` — construct a default `DaemonRequest::Run` for tests.
//!
//! Eliminates boilerplate when constructing `DaemonRequest::Run` in test code.
//! Every field gets a sensible default; override any subset with named arguments.
//!
//! # Examples
//!
//! ```rust,ignore
//! use minibox_macros::test_run;
//!
//! // All defaults
//! let req = test_run!();
//!
//! // Override specific fields
//! let req = test_run!(image: "ubuntu", tag: Some("22.04".to_string()));
//! let req = test_run!(env: vec!["FOO=bar".to_string()], privileged: true);
//! ```

/// Construct a `DaemonRequest::Run` with sensible test defaults.
///
/// All fields default to the simplest valid value (empty vecs, `None`, `false`).
/// Override any field by name.
///
/// **Edition 2024 note:** `let`-shadowing across macro hygiene boundaries
/// is broken in edition 2024. This macro uses a helper function + struct
/// update on a plain struct to work around the limitation.
///
/// The macro references `minibox_core::protocol::DaemonRequest` so it works
/// from any crate that depends on `minibox-core`.
#[macro_export]
macro_rules! test_run {
    ($($field:ident : $val:expr),* $(,)?) => {{
        let defaults = minibox_core::protocol::TestRunDefaults::default();
        let overrides = minibox_core::protocol::TestRunDefaults {
            $($field: $val,)*
            ..defaults
        };
        overrides.into_request()
    }};
}
