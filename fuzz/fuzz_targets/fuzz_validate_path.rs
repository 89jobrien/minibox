#![no_main]
//! Fuzz target: feed arbitrary path strings to `ValidatedPath::new`.
//!
//! Uses a temporary directory as the base so canonicalization succeeds.
//! The invariant: must never panic regardless of input path.

use libfuzzer_sys::fuzz_target;
use minibox_core::path::ValidatedPath;
use std::path::Path;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let base = std::env::temp_dir();
        let _ = ValidatedPath::new(Path::new(s), &base);
    }
});
