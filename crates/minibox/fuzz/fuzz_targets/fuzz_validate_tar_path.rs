#![no_main]
//! Fuzz target: interpret arbitrary bytes as a filesystem path and feed to
//! `validate_layer_path`.
//!
//! The invariant: the function must never panic regardless of input.
//! Both Ok and Err are valid outcomes; the fuzzer is hunting for panics in
//! path component iteration, canonicalization, and comparison logic.

use libfuzzer_sys::fuzz_target;
use minibox_core::image::layer::validate_layer_path;
use std::path::Path;

fuzz_target!(|data: &[u8]| {
    // Paths on real filesystems are byte strings; construct the Path directly
    // from raw bytes without requiring valid UTF-8.
    #[cfg(unix)]
    {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let path = Path::new(OsStr::from_bytes(data));
        let _ = validate_layer_path(path);
    }
    #[cfg(not(unix))]
    {
        // On non-Unix platforms paths must be valid UTF-8; skip invalid bytes.
        if let Ok(s) = std::str::from_utf8(data) {
            let _ = validate_layer_path(Path::new(s));
        }
    }
});
