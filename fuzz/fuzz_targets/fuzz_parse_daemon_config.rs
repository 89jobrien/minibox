#![no_main]
//! Fuzz target: feed arbitrary UTF-8 strings as TOML to `DaemonConfig`
//! deserialization.
//!
//! The invariant: deserialization must never panic regardless of input.
//! Ok and Err are both valid outcomes.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = toml::from_str::<miniboxd::config::DaemonConfig>(s);
    }
});
