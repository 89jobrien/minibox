//! `adapt!` -- implement both `AsAny` and `Default` for one or more types.
//!
//! Equivalent to calling [`as_any!`] and [`default_new!`] with the same type
//! list. Only valid for types whose `new()` takes no arguments.
//!
//! # Examples
//!
//! ```rust,ignore
//! adapt!(ColimaRegistry, ColimaFilesystem, ColimaLimiter, ColimaRuntime);
//! ```

/// Implements both `AsAny` and `Default` for one or more adapter types.
#[macro_export]
macro_rules! adapt {
    ($($t:ty),+ $(,)?) => {
        $crate::as_any!($($t),+);
        $crate::default_new!($($t),+);
    };
}
