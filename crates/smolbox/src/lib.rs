//! smolbox -- smolvm and krun adapter suites for minibox.
//!
//! This crate houses the lightweight VM adapter backends:
//!
//! - [`smolvm`] -- SmolVM CLI adapter (delegates to `smolvm machine run`)
//! - [`krun`] -- libkrun FFI adapter (delegates to smolvm/libkrun microVMs)
//! - [`preflight`] -- smolvm binary detection and version checking

pub mod krun;
pub mod preflight;
pub mod smolvm;
