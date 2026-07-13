#![no_main]
//! Fuzz target: feed arbitrary bytes as a manifest JSON body to
//! `ManifestResponse::parse` with both "single" and "list" media types.
//!
//! The invariant: the function must never panic regardless of input.
//! Ok and Err are both valid; the fuzzer hunts for panics in serde_json
//! deserialization of the OciManifest / ManifestList shapes.

use libfuzzer_sys::fuzz_target;
use minibox_core::image::manifest::ManifestResponse;

// Split the input: first byte selects the media-type variant; remaining bytes
// are treated as the JSON body. This lets the fuzzer explore both parse paths
// from a single harness without duplicating the corpus.
fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let (selector, body) = data.split_at(1);
    let media_type = if selector[0] & 1 == 0 {
        // single-arch manifest
        "application/vnd.oci.image.manifest.v1+json"
    } else {
        // manifest list / image index
        "application/vnd.docker.distribution.manifest.list.v2+json"
    };
    let _ = ManifestResponse::parse(body, media_type);
});
