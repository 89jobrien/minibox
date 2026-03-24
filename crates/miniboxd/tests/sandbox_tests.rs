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

#[test]
#[ignore]
fn sandbox_filesystem_write_read() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute("alpine", &["sh", "-c", "echo data > /tmp/t && cat /tmp/t"]);
    assert_eq!(result.exit_code, 0, "write+read should succeed");
    assert_eq!(
        result.stdout.trim(),
        "data",
        "should read back what was written"
    );
}

#[test]
#[ignore]
fn sandbox_sequential_runs_isolated() {
    require_sandbox_caps();
    let mut sb = sandbox();

    // Run 1: write a file
    let r1 = sb.execute(
        "alpine",
        &["sh", "-c", "echo secret > /tmp/state && echo ok"],
    );
    assert_eq!(r1.exit_code, 0, "first run should succeed");
    assert_eq!(r1.stdout.trim(), "ok");

    // Run 2: try to read that file — should fail (fresh overlay)
    let r2 = sb.execute("alpine", &["sh", "-c", "cat /tmp/state"]);
    assert_ne!(
        r2.exit_code, 0,
        "second run should fail: /tmp/state should not exist in a fresh container"
    );
}

#[test]
#[ignore]
fn sandbox_concurrent_runs_isolated() {
    require_sandbox_caps();
    let mut sb = sandbox();

    // Spawn two containers that each write a unique value, sleep, then read it back
    let mut child_a = sb.spawn_container(
        "alpine",
        &["sh", "-c", "echo AAA > /tmp/id && sleep 1 && cat /tmp/id"],
    );
    let mut child_b = sb.spawn_container(
        "alpine",
        &["sh", "-c", "echo BBB > /tmp/id && sleep 1 && cat /tmp/id"],
    );

    // Wait for both and capture output
    let out_a = child_a.wait_with_output().expect("child_a wait failed");
    let out_b = child_b.wait_with_output().expect("child_b wait failed");

    let stdout_a = String::from_utf8_lossy(&out_a.stdout);
    let stdout_b = String::from_utf8_lossy(&out_b.stdout);

    // Each container should read its own value, not the other's
    assert!(
        stdout_a.contains("AAA"),
        "container A should read AAA, got: {stdout_a}"
    );
    assert!(
        stdout_b.contains("BBB"),
        "container B should read BBB, got: {stdout_b}"
    );
    assert!(!stdout_a.contains("BBB"), "container A should NOT see BBB");
    assert!(!stdout_b.contains("AAA"), "container B should NOT see AAA");
}

#[test]
#[ignore]
fn sandbox_oom_kill() {
    require_sandbox_caps();
    let mut sb = sandbox();

    // 16 MB memory limit, try to allocate 64 MB via /dev/zero
    let result = sb.execute_with_limits(
        "alpine",
        &[
            "sh",
            "-c",
            "dd if=/dev/zero of=/dev/shm/fill bs=1M count=64 2>&1; echo done",
        ],
        16 * 1024 * 1024,
        100,
    );

    // The process should either be OOM-killed (exit != 0) or dd should fail
    // We check both: non-zero exit AND that dd didn't fully succeed
    let succeeded_fully = result.stdout.contains("64+0 records out");
    assert!(
        result.exit_code != 0 || !succeeded_fully,
        "container should be OOM-killed or dd should fail with 16 MB limit.\n\
         exit_code: {}\nstdout: {:?}\nstderr: {:?}",
        result.exit_code,
        result.stdout,
        result.stderr
    );
}

// ---------------------------------------------------------------------------
// Python scenarios (python:3.12-alpine)
// ---------------------------------------------------------------------------

const PYTHON_IMAGE: &str = "python:3.12-alpine";

#[test]
#[ignore]
fn python_sandbox_basic_script() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute(PYTHON_IMAGE, &["python3", "-c", "print(1+1)"]);
    assert_eq!(result.exit_code, 0, "python should exit 0");
    assert_eq!(result.stdout.trim(), "2", "print(1+1) should output '2'");
}

#[test]
#[ignore]
fn python_sandbox_exception_captured() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute(PYTHON_IMAGE, &["python3", "-c", "raise ValueError('oops')"]);
    assert_ne!(
        result.exit_code, 0,
        "exception should produce non-zero exit"
    );
    assert!(
        result.stderr.contains("ValueError"),
        "stderr should contain 'ValueError', got: {:?}",
        result.stderr
    );
}

#[test]
#[ignore]
fn python_sandbox_json_output() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute(
        PYTHON_IMAGE,
        &["python3", "-c", "import json; print(json.dumps({'r': 42}))"],
    );
    assert_eq!(result.exit_code, 0, "json output should exit 0");

    let parsed: serde_json::Value =
        serde_json::from_str(result.stdout.trim()).unwrap_or_else(|e| {
            panic!(
                "stdout should be valid JSON: {e}\nstdout: {:?}",
                result.stdout
            )
        });
    assert_eq!(parsed["r"], 42, "JSON should contain r=42, got: {parsed}");
}

#[test]
#[ignore]
fn python_sandbox_multiline_output() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute(
        PYTHON_IMAGE,
        &["python3", "-c", "for i in range(5): print(i)"],
    );
    assert_eq!(result.exit_code, 0, "multiline script should exit 0");
    let lines: Vec<&str> = result.stdout.trim().lines().collect();
    assert_eq!(lines, vec!["0", "1", "2", "3", "4"], "should print 0-4");
}

#[test]
#[ignore]
fn python_sandbox_network_blocked() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute(
        PYTHON_IMAGE,
        &[
            "python3",
            "-c",
            "import urllib.request; urllib.request.urlopen('http://1.1.1.1', timeout=2)",
        ],
    );
    assert_ne!(
        result.exit_code, 0,
        "network access should fail in isolated container"
    );
    // Python should raise OSError or URLError
    assert!(
        result.stderr.contains("Error") || result.stderr.contains("error"),
        "stderr should mention an error.\nstderr: {:?}",
        result.stderr
    );
}
