//! `macro_rules!` macros for reducing adapter boilerplate in minibox.
//!
//! This crate contains declarative (`macro_rules!`) macros only — not
//! procedural macros — despite being registered as a proc-macro crate in
//! `Cargo.toml` for historical reasons. All macros are re-exported with
//! `#[macro_export]` and are available to downstream crates via the crate root.
//!
//! # Available macros
//!
//! - [`as_any!`] — implement `AsAny` for one or more types
//! - [`default_new!`] — implement `Default` via `Self::new()` for one or more types
//! - [`adapt!`] — implement both `AsAny` and `Default` for one or more types
//!
//! # Call-site resolution note
//!
//! The `as_any!` macro references `crate::domain::AsAny`. In `macro_rules!`,
//! `crate` resolves at the *call site*, not the defining crate. When called
//! from `minibox-lib`, `crate` correctly expands to `minibox_lib`, so the
//! path resolves to `minibox_lib::domain::AsAny`. Using `$crate` would
//! incorrectly resolve to `minibox_macros`, which does not define `AsAny`.
//! Clippy warns about this pattern (`crate_in_macro_def`); the warning is
//! suppressed with `#[allow]` — do not change it to `$crate`.

/// Implement `crate::domain::AsAny` for one or more types.
///
/// This allows trait objects to be downcast back to their concrete type at
/// runtime via `std::any::Any`. The path `crate::domain::AsAny` resolves at
/// the **call site** (i.e., in `minibox-lib`), not in this defining crate.
/// See the crate-level documentation for the full explanation.
///
/// # Example
/// ```rust,ignore
/// as_any!(DockerHubRegistry);
/// as_any!(WslRuntime, WslFilesystem, WslLimiter);
/// ```
// `crate::domain::AsAny` is intentional: in macro_rules!, `crate` resolves at
// the call site, so this expands to `minibox_lib::domain::AsAny` when invoked
// from minibox-lib. Using `$crate` here would wrongly resolve to minibox-macros,
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

/// Implement `Default` by delegating to `Self::new()`.
///
/// Generates `impl Default for T { fn default() -> Self { Self::new() } }` for
/// each listed type. Only valid for types whose `new()` takes no arguments.
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

/// Implement both `AsAny` and `Default` for one or more types.
///
/// Equivalent to calling [`as_any!`] and [`default_new!`] with the same type
/// list. Only valid for types whose `new()` takes no arguments. Use this as
/// the single call-site macro for adapter types that need both traits.
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
