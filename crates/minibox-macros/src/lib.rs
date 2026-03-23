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
//! - [`provide!`] — generate provider constructors in `minibox-llm`
//! - [`require_capability!`] — skip tests when a probed host capability is absent
//!
//! # Call-site resolution note
//!
//! The `as_any!` macro references `crate::domain::AsAny`. In `macro_rules!`,
//! `crate` resolves at the *call site*, not the defining crate. When called
//! from `linuxbox`, `crate` correctly expands to `linuxbox`, so the
//! path resolves to `linuxbox::domain::AsAny`. Using `$crate` would
//! incorrectly resolve to `minibox_macros`, which does not define `AsAny`.
//! Clippy warns about this pattern (`crate_in_macro_def`); the warning is
//! suppressed with `#[allow]` — do not change it to `$crate`.

/// Implement `crate::domain::AsAny` for one or more types.
///
/// This allows trait objects to be downcast back to their concrete type at
/// runtime via `std::any::Any`. The path `crate::domain::AsAny` resolves at
/// the **call site** (i.e., in `linuxbox`), not in this defining crate.
/// See the crate-level documentation for the full explanation.
///
/// # Example
/// ```rust,ignore
/// as_any!(DockerHubRegistry);
/// as_any!(WslRuntime, WslFilesystem, WslLimiter);
/// ```
// `crate::domain::AsAny` is intentional: in macro_rules!, `crate` resolves at
// the call site, so this expands to `linuxbox::domain::AsAny` when invoked
// from linuxbox. Using `$crate` here would wrongly resolve to minibox-macros,
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

/// Generate `from_env()`, `from_env_with_config()`, and test-only `from_key()`
/// constructors for an LLM provider type.
///
/// This macro is intended for use inside `minibox-llm` provider modules. It
/// intentionally references `crate::ProviderConfig` at the call site, so it
/// expands against `minibox_llm`, not `minibox_macros`.
///
/// # Example
/// ```rust,ignore
/// minibox_macros::provide!(OpenAiProvider, "OPENAI_API_KEY", "gpt-4.1");
/// ```
#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! provide {
    ($provider:ty, $env_var:expr, $default_model:expr) => {
        impl $provider {
            /// Construct this provider from the environment using default HTTP timeouts.
            ///
            /// Returns `None` if the required API key environment variable is not set.
            pub fn from_env() -> Option<Self> {
                Self::from_env_with_config(&crate::ProviderConfig::default())
            }

            /// Construct this provider from the environment with explicit HTTP configuration.
            ///
            /// Returns `None` if the required API key environment variable is not set.
            pub fn from_env_with_config(config: &crate::ProviderConfig) -> Option<Self> {
                ::std::env::var($env_var)
                    .ok()
                    .map(|k| Self::with_config(k, $default_model.to_string(), config))
            }

            /// Test helper — inject a key without reading the environment.
            #[cfg(test)]
            pub(crate) fn from_key(key: String) -> Self {
                Self::new(key, $default_model.to_string())
            }
        }
    };
}

/// Skip a test gracefully when a required host capability is absent.
///
/// If `$caps.$field` is `false`, prints a `SKIPPED: $reason` message to stderr
/// and returns from the calling function early. Intended for integration tests
/// that probe host state before executing privileged setup.
///
/// # Example
/// ```rust,ignore
/// minibox_macros::require_capability!(caps, is_root, "requires root");
/// ```
#[macro_export]
macro_rules! require_capability {
    ($caps:expr, $field:ident, $reason:expr) => {
        if !$caps.$field {
            ::std::eprintln!("SKIPPED: {}", $reason);
            return;
        }
    };
}
