#![no_main]
//! Fuzz target: feed arbitrary UTF-8 strings to `BindMount::parse_mount`.
//!
//! The invariant: the parser must never panic regardless of input.

use libfuzzer_sys::fuzz_target;
use minibox_core::domain::BindMount;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = BindMount::parse_mount(s);
    }
});
