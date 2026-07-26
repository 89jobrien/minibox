#![no_main]
//! Fuzz target: feed arbitrary bytes as both data and digest string to
//! `verify_digest`.
//!
//! The invariant: the function must never panic regardless of input.
//! Ok and Err are both valid outcomes.

use libfuzzer_sys::fuzz_target;
use minibox_core::image::layer::verify_digest;

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 {
        return;
    }
    // Split: first byte determines where to split data vs digest string.
    let split = (data[0] as usize) % data.len().max(1);
    let (payload, digest_bytes) = data.split_at(split.min(data.len()));
    if let Ok(digest_str) = std::str::from_utf8(digest_bytes) {
        let _ = verify_digest(payload, digest_str);
    }
});
