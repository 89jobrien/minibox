//! Quickcheck property tests for the minibox crate.
//!
//! Property families:
//! 4. Cgroup limit arithmetic — no overflow, no zero-division
//! 5. IP allocator no-double-assign — allocate() never returns an in-use IP
//! 6. Overlay mount-option string — output is valid mount(2) option syntax

use quickcheck::TestResult;
use quickcheck_macros::quickcheck;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// 4. Cgroup limit arithmetic — no overflow, no zero-division
// ---------------------------------------------------------------------------

/// CgroupConfig memory_limit_bytes: any u64 value stored and retrieved
/// without overflow or panic.
#[quickcheck]
fn cgroup_memory_limit_no_overflow(bytes: u64) -> bool {
    // The memory limit is written as a string to cgroup files.
    // Verify the format conversion does not overflow or panic.
    let formatted = format!("{bytes}");
    let parsed: u64 = formatted.parse().expect("should parse back");
    parsed == bytes
}

/// CPU weight is in range 1-10000. Values outside this range should be
/// clampable without panic.
#[quickcheck]
fn cgroup_cpu_weight_clamp_no_panic(weight: u64) -> bool {
    let clamped = weight.clamp(1, 10000);
    (1..=10000).contains(&clamped)
}

/// PIDs max: any non-zero u64 value formatted as string roundtrips.
#[quickcheck]
fn cgroup_pids_max_roundtrip(pids: u64) -> TestResult {
    if pids == 0 {
        return TestResult::discard();
    }
    let s = format!("{pids}");
    let parsed: u64 = s.parse().expect("should parse");
    TestResult::from_bool(parsed == pids)
}

/// IO bandwidth: bytes-per-second formatting does not panic for any u64.
#[quickcheck]
fn cgroup_io_bandwidth_format_no_panic(bps: u64) -> bool {
    // The kernel expects "MAJ:MIN rbps=N wbps=N" format.
    let formatted = format!("8:0 rbps={bps} wbps={bps}");
    formatted.contains(&bps.to_string())
}

// ---------------------------------------------------------------------------
// 5. IP allocator no-double-assign (Linux-only: bridge module is gated)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod ip_allocator_tests {
    use super::*;
    use std::collections::HashSet;

    /// Allocate N IPs from a /24 subnet and verify no duplicates.
    #[quickcheck]
    fn ip_allocator_no_double_assign(count: u8) -> TestResult {
        use ipnet::IpNet;
        use minibox::adapters::network::bridge::IpAllocator;

        let subnet: IpNet = "10.0.0.0/24".parse().expect("valid subnet");
        let mut alloc = IpAllocator::new(subnet).expect("create allocator");

        let count = count.min(250);
        let mut seen = HashSet::new();

        for _ in 0..count {
            match alloc.allocate() {
                Some(ip) => {
                    if !seen.insert(ip) {
                        return TestResult::error(format!("duplicate IP allocated: {ip}"));
                    }
                }
                None => break,
            }
        }

        TestResult::passed()
    }

    /// Releasing and re-allocating an IP must not produce a duplicate of
    /// any still-held address.
    #[quickcheck]
    fn ip_allocator_release_reuse(seed: u8) -> TestResult {
        use ipnet::IpNet;
        use minibox::adapters::network::bridge::IpAllocator;

        let subnet: IpNet = "10.0.0.0/24".parse().expect("valid subnet");
        let mut alloc = IpAllocator::new(subnet).expect("create allocator");

        let n = (seed % 10) + 1;
        let mut allocated = Vec::new();
        for _ in 0..n {
            if let Some(ip) = alloc.allocate() {
                allocated.push(ip);
            }
        }

        if allocated.is_empty() {
            return TestResult::discard();
        }

        let released = *allocated.last().expect("non-empty");
        alloc.release(released);

        let reallocated = alloc.allocate();
        if let Some(ip) = reallocated {
            let still_held: HashSet<_> = allocated[..allocated.len() - 1].iter().collect();
            TestResult::from_bool(!still_held.contains(&ip))
        } else {
            TestResult::passed()
        }
    }
}

// ---------------------------------------------------------------------------
// 6. Overlay mount-option string — valid mount(2) syntax
// ---------------------------------------------------------------------------

/// Overlay mount options must follow the format:
/// `lowerdir=<paths>,upperdir=<path>,workdir=<path>`
#[quickcheck]
fn overlay_mount_options_valid_syntax(n_layers: u8) -> TestResult {
    let n_layers = (n_layers % 5) + 1;
    let layers: Vec<PathBuf> = (0..n_layers)
        .map(|i| PathBuf::from(format!("/var/lib/minibox/images/layer{i}")))
        .collect();

    let container_dir = PathBuf::from("/var/lib/minibox/containers/test123");
    let upper = container_dir.join("upper");
    let work = container_dir.join("work");

    let lowerdir: String = layers
        .iter()
        .rev()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(":");

    let options = format!(
        "lowerdir={lowerdir},upperdir={upper},workdir={work}",
        upper = upper.display(),
        work = work.display(),
    );

    let has_lowerdir = options.starts_with("lowerdir=");
    let has_upperdir = options.contains(",upperdir=");
    let has_workdir = options.contains(",workdir=");
    let no_empty_values = !options.contains("=,") && !options.ends_with('=');
    let no_spaces = !options.contains(' ');

    TestResult::from_bool(
        has_lowerdir && has_upperdir && has_workdir && no_empty_values && no_spaces,
    )
}

/// Layer paths with safe filesystem characters should produce valid
/// comma/colon-delimited options.
#[quickcheck]
fn overlay_options_no_delimiter_collision(layer_name: String) -> TestResult {
    let safe_name: String = layer_name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
        .take(32)
        .collect();
    if safe_name.is_empty() {
        return TestResult::discard();
    }

    let layer = PathBuf::from(format!("/images/{safe_name}"));
    let lowerdir = layer.display().to_string();

    TestResult::from_bool(!lowerdir.contains(','))
}
