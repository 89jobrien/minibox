#![no_main]
//! Fuzz target: feed arbitrary UTF-8 strings to `parse_www_authenticate`.
//!
//! This parses the WWW-Authenticate header from external container registries.
//! The invariant: the function must never panic regardless of input.

use libfuzzer_sys::fuzz_target;
use minibox::adapters::ghcr::parse_www_authenticate;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = parse_www_authenticate(s);
    }
});
