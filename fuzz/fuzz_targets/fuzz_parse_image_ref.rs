#![no_main]
//! Fuzz target: interpret arbitrary bytes as a UTF-8 image reference string
//! and feed to `ImageRef::parse`.
//!
//! The invariant: the function must never panic regardless of input.
//! Both Ok and Err are valid outcomes.

use libfuzzer_sys::fuzz_target;
use minibox_core::image::reference::ImageRef;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = ImageRef::parse(s);
    }
});
