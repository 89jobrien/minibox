#![no_main]
//! Fuzz target: interpret arbitrary bytes as a UTF-8 platform string and feed
//! to `TargetPlatform::parse`.
//!
//! The invariant: the function must never panic regardless of input.
//! Both Ok and Err are valid outcomes.

use libfuzzer_sys::fuzz_target;
use minibox_core::image::manifest::TargetPlatform;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = TargetPlatform::parse(s);
    }
});
