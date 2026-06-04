#![no_main]
//! Fuzz target: feed arbitrary JSON to `parse_mounts`.
//!
//! The invariant: `parse_mounts` must never panic — Ok and Err are both
//! valid outcomes.

use libfuzzer_sys::fuzz_target;
use minibox_crux_plugin::parse_mounts;

fuzz_target!(|data: &[u8]| {
    let v: serde_json::Value = match serde_json::from_slice(data) {
        Ok(v) => v,
        Err(_) => return,
    };
    let _ = parse_mounts(&v);
});
