//! SmolVM-isolated e2e tests.
//!
//! These tests run commands inside smolvm ephemeral VMs, providing real Linux
//! kernel isolation (cgroups, namespaces, overlay FS) on macOS.
//!
//! Skipped automatically when smolvm is not installed.
//!
//! **Running:**
//! ```bash
//! cargo test -p miniboxd --test smolvm_e2e_tests
//! ```

mod helpers;

use helpers::smolvm::{smolvm_available, smolvm_run};
use serial_test::serial;

const IMAGE: &str = "alpine:3.20";

/// Skip the test if smolvm is not installed.
macro_rules! require_smolvm {
    () => {
        if !smolvm_available() {
            eprintln!("SKIPPED: smolvm not installed");
            return;
        }
    };
}

// ---------------------------------------------------------------------------
// Kernel feature tests
// ---------------------------------------------------------------------------

#[test]
#[serial(smolvm)]
fn smolvm_boots_and_returns_kernel_version() {
    require_smolvm!();
    let out = smolvm_run(IMAGE, &["uname", "-r"]);
    assert!(out.success, "uname inside VM failed: {}", out.stderr);
    assert!(
        !out.stdout.trim().is_empty(),
        "expected kernel version, got empty output"
    );
}

#[test]
#[serial(smolvm)]
fn smolvm_cgroups_v2_available() {
    require_smolvm!();
    let out = smolvm_run(IMAGE, &["mount"]);
    assert!(out.success, "mount inside VM failed: {}", out.stderr);
    assert!(
        out.stdout.contains("cgroup2"),
        "cgroups v2 not mounted in VM: {}",
        out.stdout
    );
}

#[test]
#[serial(smolvm)]
fn smolvm_overlay_fs_available() {
    require_smolvm!();
    let out = smolvm_run(IMAGE, &["cat", "/proc/filesystems"]);
    assert!(out.success, "cat /proc/filesystems failed: {}", out.stderr);
    assert!(
        out.stdout.contains("overlay"),
        "overlay FS not available in VM: {}",
        out.stdout
    );
}

#[test]
#[serial(smolvm)]
fn smolvm_namespaces_available() {
    require_smolvm!();
    let out = smolvm_run(IMAGE, &["ls", "/proc/self/ns/"]);
    assert!(out.success, "ls /proc/self/ns failed: {}", out.stderr);
    for ns in ["mnt", "pid", "net", "uts"] {
        assert!(
            out.stdout.contains(ns),
            "namespace '{ns}' not found in /proc/self/ns/: {}",
            out.stdout
        );
    }
}

// ---------------------------------------------------------------------------
// Volume and workspace tests
// ---------------------------------------------------------------------------
// Note: smolvm volume mounts (-v host:guest) create the mountpoint inside
// the VM but do not populate it with host files on macOS (virtiofs limitation
// as of smolvm 0.5.x). These tests are disabled until smolvm adds macOS
// file sharing support.
//
// When virtiofs works, re-enable with:
//   smolvm_run_workspace(IMAGE, &["test", "-f", "/workspace/Cargo.toml"])

// ---------------------------------------------------------------------------
// Network tests
// ---------------------------------------------------------------------------

#[test]
#[serial(smolvm)]
fn smolvm_dns_resolution_works() {
    require_smolvm!();
    let out = smolvm_run(
        IMAGE,
        &[
            "wget",
            "-q",
            "-O",
            "/dev/null",
            "--timeout=5",
            "http://registry-1.docker.io/v2/",
        ],
    );
    // 401 Unauthorized is expected (no auth), but confirms network + DNS work.
    // wget returns non-zero on 401, so check stderr for the response.
    assert!(
        out.success || out.stderr.contains("401") || out.stderr.contains("server returned error"),
        "DNS/network should work inside VM.\nstderr: {}",
        out.stderr
    );
}
