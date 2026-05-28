#![no_main]
//! Fuzz target: feed arbitrary bytes as a JSON-RPC Request.
//!
//! Exercises the same parse path the plugin's main loop uses.
//! The invariant: deserialization must never panic.

use libfuzzer_sys::fuzz_target;
use minibox_crux_plugin::protocol::Request;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = serde_json::from_str::<Request>(s);
    }
});
