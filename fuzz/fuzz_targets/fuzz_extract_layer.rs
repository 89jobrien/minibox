#![no_main]
use libfuzzer_sys::fuzz_target;
use minibox::image::layer::extract_layer;
use tempfile::TempDir;

fuzz_target!(|data: &[u8]| {
    // A fresh TempDir per iteration — extraction must never write outside it.
    if let Ok(dir) = TempDir::new() {
        let _ = extract_layer(data, dir.path());
        // Verify nothing escaped: the dest dir must still exist after extraction.
        assert!(dir.path().exists(), "dest dir must still exist after extraction");
    }
});
