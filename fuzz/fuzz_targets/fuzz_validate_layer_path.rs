#![no_main]
use libfuzzer_sys::fuzz_target;
use std::path::Path;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let path = Path::new(s);
        // Must never panic — only Ok or Err
        let _ = minibox_oci::image::layer::validate_layer_path(path);
    }
});
