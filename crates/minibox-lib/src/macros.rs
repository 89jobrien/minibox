//! Utility macros for reducing adapter boilerplate.
//!
//! # Available macros
//!
//! - [`as_any!`] — implement `AsAny` for one or more types
//! - [`default_new!`] — implement `Default` via `Self::new()` for one or more types
//! - [`adapt!`] — implement both `AsAny` and `Default` for one or more types

/// Implement [`crate::domain::AsAny`] for one or more types.
///
/// # Example
/// ```rust,ignore
/// as_any!(DockerHubRegistry);
/// as_any!(WslRuntime, WslFilesystem, WslLimiter);
/// ```
#[macro_export]
macro_rules! as_any {
    ($($t:ty),+ $(,)?) => {
        $(
            impl $crate::domain::AsAny for $t {
                fn as_any(&self) -> &dyn ::std::any::Any {
                    self
                }
            }
        )+
    };
}

/// Implement `Default` by delegating to `Self::new()`.
///
/// Only valid for types whose `new()` takes no arguments.
///
/// # Example
/// ```rust,ignore
/// default_new!(CgroupV2Limiter, OverlayFilesystem);
/// ```
#[macro_export]
macro_rules! default_new {
    ($($t:ty),+ $(,)?) => {
        $(
            impl Default for $t {
                fn default() -> Self {
                    Self::new()
                }
            }
        )+
    };
}

/// Implement both [`crate::domain::AsAny`] and `Default` for one or more types.
///
/// Equivalent to calling [`as_any!`] and [`default_new!`] with the same list.
/// Only valid for types whose `new()` takes no arguments.
///
/// # Example
/// ```rust,ignore
/// adapt!(ColimaRegistry, ColimaFilesystem, ColimaLimiter, ColimaRuntime);
/// ```
#[macro_export]
macro_rules! adapt {
    ($($t:ty),+ $(,)?) => {
        $crate::as_any!($($t),+);
        $crate::default_new!($($t),+);
    };
}
