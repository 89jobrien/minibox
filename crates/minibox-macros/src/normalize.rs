//! Normalization macros for filesystem path components.
//!
//! These macros replace characters that are invalid or inconvenient in
//! filesystem paths (`/`, `:`) with `_`, and provide the reverse operation
//! for digest strings.
//!
//! - [`normalize_name!`] -- replace `/` with `_`
//! - [`normalize_digest!`] -- replace `:` with `_`
//! - [`normalize!`] -- replace both `/` and `:` with `_`
//! - [`denormalize_digest!`] -- reverse `normalize_digest!` (replace `_` with `:`)

/// Normalize an image name string for use as a filesystem path component.
///
/// Replaces `/` with `_` (e.g. `"library/alpine"` -> `"library_alpine"`).
///
/// # Examples
///
/// ```rust
/// use minibox_macros::normalize_name;
/// assert_eq!(normalize_name!("library/alpine"), "library_alpine");
/// assert_eq!(normalize_name!("ghcr.io/org/image"), "ghcr.io_org_image");
/// ```
#[macro_export]
macro_rules! normalize_name {
    ($s:expr) => {
        $s.replace('/', "_")
    };
}

/// Normalize a digest string for use as a filesystem path component.
///
/// Replaces `:` with `_` (e.g. `"sha256:abc123"` -> `"sha256_abc123"`).
///
/// # Examples
///
/// ```rust
/// use minibox_macros::normalize_digest;
/// assert_eq!(normalize_digest!("sha256:abc123"), "sha256_abc123");
/// ```
#[macro_export]
macro_rules! normalize_digest {
    ($s:expr) => {
        $s.replace(':', "_")
    };
}

/// Normalize a string for use as a filesystem path component, replacing both
/// `/` and `:` with `_`.
///
/// # Examples
///
/// ```rust
/// use minibox_macros::normalize;
/// assert_eq!(normalize!("ghcr.io/org/image:stable"), "ghcr.io_org_image_stable");
/// ```
#[macro_export]
macro_rules! normalize {
    ($s:expr) => {
        $s.replace(['/', ':'], "_")
    };
}

/// Recover a digest string from a filesystem path component.
///
/// Reverses [`normalize_digest!`] by replacing `_` with `:`.
///
/// # Examples
///
/// ```rust
/// use minibox_macros::denormalize_digest;
/// assert_eq!(denormalize_digest!("sha256_abc123"), "sha256:abc123");
/// ```
#[macro_export]
macro_rules! denormalize_digest {
    ($s:expr) => {
        $s.replace('_', ":")
    };
}
