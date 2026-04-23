#![no_main]
use libfuzzer_sys::fuzz_target;
use minibox_core::protocol::decode_response;

fuzz_target!(|data: &[u8]| {
    // Must never panic — only Ok or Err are acceptable outcomes
    let _ = decode_response(data);
});
