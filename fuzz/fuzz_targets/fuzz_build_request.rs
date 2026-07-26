#![no_main]
//! Fuzz target: feed arbitrary bytes to `build_request`.
//!
//! First byte selects the handler index; remaining bytes are treated as the
//! JSON input body. The invariant: `build_request` must never panic — Ok and
//! Err are both valid outcomes.

use libfuzzer_sys::fuzz_target;
use minibox_crux_plugin::build_request;

const HANDLERS: &[&str] = &[
    "minibox::container::run",
    "minibox::container::stop",
    "minibox::container::pause",
    "minibox::container::resume",
    "minibox::container::rm",
    "minibox::container::exec",
    "minibox::container::ps",
    "minibox::container::logs",
    "minibox::image::pull",
    "minibox::image::build",
    "minibox::image::push",
    "minibox::image::ls",
    "minibox::image::rm",
];

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let (selector, body) = data.split_at(1);
    let handler = HANDLERS[selector[0] as usize % HANDLERS.len()];
    let input: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return, // Not valid JSON — nothing useful to test
    };
    let _ = build_request(handler, &input);
});
