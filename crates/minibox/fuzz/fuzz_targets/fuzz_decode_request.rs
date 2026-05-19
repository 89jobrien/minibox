#![no_main]

use libfuzzer_sys::fuzz_target;
use minibox_core::protocol::decode_request;

// Invariant: any byte sequence either decodes to a valid DaemonRequest
// or returns Err — it must never panic, abort, or access out-of-bounds memory.
fuzz_target!(|data: &[u8]| {
    let _ = decode_request(data);
});
