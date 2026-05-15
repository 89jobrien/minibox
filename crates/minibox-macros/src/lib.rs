//! `macro_rules!` macros for reducing adapter boilerplate in minibox.
//!
//! This crate contains declarative (`macro_rules!`) macros only — not
//! procedural macros — despite being registered as a proc-macro crate in
//! `Cargo.toml` for historical reasons. All macros are re-exported with
//! `#[macro_export]` and are available to downstream crates via the crate root.

//! # Available macros
//!
//! - [`as_any!`] — implement `AsAny` for one or more types
//! - [`default_new!`] — implement `Default` via `Self::new()` for one or more types
//! - [`adapt!`] — implement both `AsAny` and `Default` for one or more types
//! - [`provide!`] — generate provider constructors for LLM adapters
//! - [`require_capability!`] — skip tests when a probed host capability is absent
//! - [`normalize_name!`] — replace `/` with `_` for filesystem path components
//! - [`normalize_digest!`] — replace `:` with `_` for filesystem path components
//! - [`normalize!`] — replace both `/` and `:` with `_`
//! - [`denormalize_digest!`] — reverse `normalize_digest!` (replace `_` with `:`)
//! - [`test_run!`] — construct a default `DaemonRequest::Run` for tests
//!
//! # Call-site resolution note
//!
//! The `as_any!` macro references `crate::domain::AsAny`. In `macro_rules!`,
//! `crate` resolves at the *call site*, not the defining crate. When called
//! from `minibox`, `crate` correctly expands to `minibox`, so the
//! path resolves to `minibox::domain::AsAny`. Using `$crate` would
//! incorrectly resolve to `minibox_macros`, which does not define `AsAny`.
//! Clippy warns about this pattern (`crate_in_macro_def`); the warning is
//! suppressed with `#[allow]` — do not change it to `$crate`.

mod adapt;
mod as_any;
mod default_new;
mod normalize;
mod provide;
mod require_capability;
mod test_run;
