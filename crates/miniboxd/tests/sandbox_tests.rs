//! Sandbox contract tests: validates minibox as an AI agent code-execution sandbox.
//!
//! Tests exercise real images (alpine, python:3.12-alpine) in real containers,
//! verifying output capture, exit codes, network isolation, filesystem containment,
//! resource limits, and concurrency isolation.
//!
//! **Requirements:** Linux, root, cgroups v2, network access (Docker Hub)
//!
//! **Running:**
//! ```bash
//! just test-sandbox
//! ```

#![cfg(target_os = "linux")]

mod helpers;
use helpers::SandboxClient;

use linuxbox::preflight;
use linuxbox::require_capability;
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Shared fixture — one daemon + cached images for all tests
// ---------------------------------------------------------------------------

static SANDBOX: OnceLock<Mutex<SandboxClient>> = OnceLock::new();

fn sandbox() -> std::sync::MutexGuard<'static, SandboxClient> {
    SANDBOX
        .get_or_init(|| Mutex::new(SandboxClient::start()))
        .lock()
        .unwrap_or_else(|e| e.into_inner()) // recover from poison
}

/// Gate: skip all sandbox tests unless root + cgroups v2.
fn require_sandbox_caps() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");
}

// ---------------------------------------------------------------------------
// Shell scenarios (alpine)
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn sandbox_stdout_captured() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute("alpine", &["echo", "hello"]);
    assert_eq!(result.exit_code, 0, "echo should exit 0");
    assert_eq!(
        result.stdout.trim(),
        "hello",
        "stdout should capture echo output"
    );
}

#[test]
#[ignore]
fn sandbox_stderr_captured() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute("alpine", &["sh", "-c", "echo err >&2"]);
    assert!(
        result.stderr.contains("err"),
        "stderr should contain 'err', got: {:?}",
        result.stderr
    );
}

#[test]
#[ignore]
fn sandbox_exit_code_zero() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute("alpine", &["true"]);
    assert_eq!(result.exit_code, 0, "/bin/true should exit 0");
}

#[test]
#[ignore]
fn sandbox_nonzero_exit_code() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute("alpine", &["sh", "-c", "exit 42"]);
    assert_eq!(
        result.exit_code, 42,
        "exit 42 should propagate as exit code 42"
    );
}

#[test]
#[ignore]
fn sandbox_large_output() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute("alpine", &["seq", "1", "10000"]);
    assert_eq!(result.exit_code, 0, "seq should exit 0");
    let line_count = result.stdout.lines().count();
    assert_eq!(
        line_count, 10000,
        "should capture all 10000 lines, got {line_count}"
    );
}

#[test]
#[ignore]
fn sandbox_network_isolated() {
    require_sandbox_caps();
    let mut sb = sandbox();
    // wget should fail — container has NetworkMode::None (isolated namespace, no interfaces)
    let result = sb.execute(
        "alpine",
        &["sh", "-c", "wget -T 2 http://1.1.1.1/ 2>&1; exit 0"],
    );
    // Container exits 0 (we forced it), but wget output should show a network error
    assert!(
        result.stdout.contains("Network unreachable")
            || result.stdout.contains("Connection timed out")
            || result.stdout.contains("bad address")
            || result.stderr.contains("Network unreachable")
            || result.stderr.contains("bad address"),
        "wget should fail with a network error.\nstdout: {:?}\nstderr: {:?}",
        result.stdout,
        result.stderr
    );
}
