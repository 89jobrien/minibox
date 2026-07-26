#![no_main]
//! Fuzz target: deserialize arbitrary JSON into `NetworkConfig`.
//!
//! The invariant: serde deserialization must never panic.

use libfuzzer_sys::fuzz_target;
use minibox_core::domain::{NetworkConfig, NetworkMode, PortMapping};

fuzz_target!(|data: &[u8]| {
    let _ = serde_json::from_slice::<NetworkConfig>(data);
    let _ = serde_json::from_slice::<NetworkMode>(data);
    let _ = serde_json::from_slice::<PortMapping>(data);
});
