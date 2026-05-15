//! `as_any!` -- implement `AsAny` for one or more types.
//!
//! Allows trait objects to be downcast back to their concrete type at runtime
//! via `std::any::Any`. The path `crate::domain::AsAny` resolves at the
//! **call site** (i.e., in `minibox`), not in this defining crate.
//!
//! # Examples
//!
//! ```rust,ignore
//! as_any!(DockerHubRegistry);
//! as_any!(WslRuntime, WslFilesystem, WslLimiter);
//! ```

// `crate::domain::AsAny` is intentional: in macro_rules!, `crate` resolves at
// the call site, so this expands to `minibox::domain::AsAny` when invoked
// from minibox. Using `$crate` here would wrongly resolve to minibox-macros,
// which does not export `AsAny`. Suppressing the clippy lint is correct here.
#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! as_any {
    ($($t:ty),+ $(,)?) => {
        $(
            impl crate::domain::AsAny for $t {
                fn as_any(&self) -> &dyn ::std::any::Any {
                    self
                }
            }
        )+
    };
}
