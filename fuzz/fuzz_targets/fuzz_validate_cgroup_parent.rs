#![no_main]
//! Fuzz target: feed arbitrary UTF-8 strings to `validate_cgroup_parent`.
//!
//! This validates user-supplied cgroup paths — a security boundary that
//! prevents directory traversal outside /sys/fs/cgroup/.
//! The invariant: the function must never panic regardless of input.
//!
//! Linux-only: the `container` module is gated behind `cfg(target_os = "linux")`.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::str::from_utf8(data) {
            let _ = minibox::container::cgroups::validate_cgroup_parent(s);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = data;
});
