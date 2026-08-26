//! Test macros for serialized mutation of process environment variables.

/// Set an environment variable in test code (Rust 2024 requires `unsafe`).
///
/// Wraps `std::env::set_var` in an `unsafe` block with a standardised
/// SAFETY comment, reducing boilerplate and rustqual UNSAFE findings in
/// test modules.
///
/// # Usage
///
/// ```ignore
/// unsafe_set_var!("MINIBOX_SOCKET_PATH", "/custom/path");
/// ```
#[macro_export]
macro_rules! unsafe_set_var {
    ($key:expr, $val:expr) => {
        // SAFETY: caller holds an env mutex; no other thread mutates
        // the process-wide environment while the lock is held.
        unsafe { std::env::set_var($key, $val) }
    };
}

/// Restore an environment variable to its previous value in test code.
///
/// Takes an `Option<String>` (from a prior `std::env::var(...).ok()` call)
/// and either sets or removes the variable accordingly.
///
/// # Usage
///
/// ```ignore
/// let prev = std::env::var("MY_VAR").ok();
/// // ... test code ...
/// unsafe_restore_var!("MY_VAR", prev);
/// ```
#[macro_export]
macro_rules! unsafe_restore_var {
    ($key:expr, $prev:expr) => {
        // SAFETY: caller holds an env mutex; no other thread mutates
        // the process-wide environment while the lock is held.
        match $prev {
            Some(v) => unsafe { std::env::set_var($key, v) },
            None => unsafe { std::env::remove_var($key) },
        }
    };
}

/// Remove an environment variable in test code (Rust 2024 requires `unsafe`).
///
/// Wraps `std::env::remove_var` in an `unsafe` block with a standardised
/// SAFETY comment.
///
/// # Usage
///
/// ```ignore
/// unsafe_remove_var!("MINIBOX_SOCKET_PATH");
/// ```
#[macro_export]
macro_rules! unsafe_remove_var {
    ($key:expr) => {
        // SAFETY: caller holds an env mutex; no other thread mutates
        // the process-wide environment while the lock is held.
        unsafe { std::env::remove_var($key) }
    };
}
