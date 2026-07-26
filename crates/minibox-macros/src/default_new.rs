//! `default_new!` -- implement `Default` by delegating to `Self::new()`.
//!
//! Generates `impl Default for T { fn default() -> Self { Self::new() } }` for
//! each listed type. Only valid for types whose `new()` takes no arguments.
//!
//! # Examples
//!
//! ```rust,ignore
//! default_new!(CgroupV2Limiter, OverlayFilesystem);
//! ```

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
