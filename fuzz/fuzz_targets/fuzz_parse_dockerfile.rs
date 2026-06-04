#![no_main]
//! Fuzz target: feed arbitrary UTF-8 strings to `dockerfile::parse`.
//!
//! The invariant: the parser must never panic regardless of input.
//! Ok and Err are both valid outcomes.

use libfuzzer_sys::fuzz_target;
use minibox_core::image::dockerfile;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = dockerfile::parse(s);
    }
});
