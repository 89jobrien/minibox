#![no_main]
//! Fuzz target: feed arbitrary bytes as a gzip-compressed tar stream to
//! `extract_layer`.
//!
//! The invariant: the function must never panic regardless of input.
//! Returning `Ok` or `Err` are both valid outcomes — the fuzzer is hunting
//! for panics, OOMs, and UB in the tar/gzip parsing and path-validation logic.

use libfuzzer_sys::fuzz_target;
use minibox_core::image::layer::extract_layer;

fuzz_target!(|data: &[u8]| {
    let dir = match tempfile::TempDir::new() {
        Ok(d) => d,
        Err(_) => return,
    };
    // Ignore Ok/Err — a clean error is expected for most random inputs.
    let _ = extract_layer(&mut data.as_ref(), dir.path());
});
