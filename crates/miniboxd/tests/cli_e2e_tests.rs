#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::redundant_field_names,
    clippy::uninlined_format_args,
    clippy::redundant_clone,
    clippy::redundant_closure,
    clippy::single_char_pattern,
    clippy::unwrap_in_result,
    clippy::collapsible_if,
    clippy::match_same_arms,
    clippy::only_used_in_recursion,
    clippy::used_underscore_binding,
    clippy::map_unwrap_or,
    clippy::manual_assert,
    clippy::as_ptr_cast_mut,
    clippy::ptr_as_ptr,
    clippy::must_use_candidate,
    clippy::used_underscore_items,
    clippy::missing_const_for_fn,
    clippy::manual_string_new,
    clippy::semicolon_if_nothing_returned,
    clippy::unreadable_literal,
    clippy::default_constructed_unit_structs,
    clippy::ref_as_ptr,
    clippy::allow_attributes_without_reason,
    clippy::redundant_closure_for_method_calls,
    clippy::needless_raw_string_hashes,
    clippy::manual_is_variant_and,
    clippy::ignore_without_reason,
    clippy::default_trait_access,
    clippy::cast_lossless,
    clippy::match_wild_err_arm,
    clippy::format_push_string,
    clippy::bool_assert_comparison,
    clippy::struct_excessive_bools,
    clippy::duration_suboptimal_units,
    clippy::unnecessary_map_or
)]
//! CLI end-to-end tests.
//!
//! These tests start a real miniboxd and exercise the `mbx` CLI binary against
//! it. Cross-platform: no root, no cgroups, no namespaces required.
//!
//! **Running:**
//! ```bash
//! cargo build --release -p miniboxd -p minibox-cli
//! MINIBOX_E2E_PULL=1 cargo test -p miniboxd --test cli_e2e_tests
//! ```
//!
//! Tests use release binaries (via `find_binary` which prefers release/).
//! The daemon's stderr is sent to /dev/null to avoid pipe-buffer deadlocks
//! when debug logging fills the 64KB OS pipe buffer during slow operations
//! like image pulls.

mod helpers;

use helpers::{find_binary, poll_until};
use serial_test::serial;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Cross-platform CLI fixture (no cgroups, no root)
// ---------------------------------------------------------------------------

struct CliFixture {
    daemon: Option<Child>,
    socket_path: std::path::PathBuf,
    cli_bin: std::path::PathBuf,
    _data_dir: TempDir,
    _run_dir: TempDir,
}

impl CliFixture {
    fn start() -> Self {
        let data_dir = TempDir::with_prefix("minibox-cli-data-").expect("create temp data dir");
        let run_dir = TempDir::with_prefix("minibox-cli-run-").expect("create temp run dir");
        let socket_path = run_dir.path().join("miniboxd.sock");

        let daemon_bin = find_binary("miniboxd");
        let cli_bin = find_binary("mbx");

        // Use Stdio::null() for stdout/stderr to prevent pipe-buffer
        // deadlocks: debug log output during pull can exceed the 64KB
        // OS pipe buffer, blocking the daemon if nobody drains the pipe.
        let daemon = Command::new(&daemon_bin)
            .env("MINIBOX_DATA_DIR", data_dir.path())
            .env("MINIBOX_RUN_DIR", run_dir.path())
            .env("MINIBOX_SOCKET_PATH", &socket_path)
            .env("MINIBOX_METRICS_ADDR", "127.0.0.1:0")
            .env("RUST_LOG", "miniboxd=info")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to start miniboxd at {daemon_bin:?}: {e}"));

        let sock = socket_path.clone();
        let started = poll_until(
            Duration::from_secs(10),
            Duration::from_millis(100),
            move || sock.exists(),
        );
        if !started {
            panic!("miniboxd did not create socket within 10s at {socket_path:?}");
        }

        Self {
            daemon: Some(daemon),
            socket_path,
            cli_bin,
            _data_dir: data_dir,
            _run_dir: run_dir,
        }
    }

    /// Run the CLI with the given args and return (success, stdout, stderr).
    ///
    /// Times out after 30 seconds to prevent hangs from adapter issues.
    fn run(&self, args: &[&str]) -> (bool, String, String) {
        let mut child = Command::new(&self.cli_bin)
            .env("MINIBOX_SOCKET_PATH", &self.socket_path)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn CLI {:?}: {e}", args));

        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if std::time::Instant::now() > deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        panic!("CLI command {:?} timed out after 30s", args);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => panic!("try_wait failed: {e}"),
            }
        }

        let output = child.wait_with_output().expect("wait_with_output");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        (output.status.success(), stdout, stderr)
    }
}

impl Drop for CliFixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ---------------------------------------------------------------------------
// Help / version
// ---------------------------------------------------------------------------

#[test]
fn cli_help_exits_zero() {
    let cli_bin = find_binary("mbx");
    let output = Command::new(&cli_bin)
        .arg("--help")
        .output()
        .expect("run --help");
    assert!(output.status.success(), "--help should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Usage:"),
        "help should contain 'Usage:', got: {stdout}"
    );
}

#[test]
fn cli_version_exits_zero() {
    let cli_bin = find_binary("mbx");
    let output = Command::new(&cli_bin)
        .arg("--version")
        .output()
        .expect("run --version");
    assert!(output.status.success(), "--version should exit 0");
}

// ---------------------------------------------------------------------------
// ps on empty daemon
// ---------------------------------------------------------------------------

#[test]
#[serial(cli_daemon)]
fn cli_ps_empty_daemon() {
    let fixture = CliFixture::start();
    let (success, stdout, stderr) = fixture.run(&["ps"]);
    assert!(
        success,
        "ps should succeed on empty daemon.\nstdout: {stdout}\nstderr: {stderr}"
    );
    // Should show a header but no container rows
    assert!(
        stdout.contains("CONTAINER") || stdout.contains("no containers"),
        "ps output should contain header or 'no containers', got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// Pull tests (require MINIBOX_E2E_PULL=1 — adapter must be able to pull)
// ---------------------------------------------------------------------------

fn pull_tests_enabled() -> bool {
    std::env::var("MINIBOX_E2E_PULL").map_or(false, |v| v == "1")
}

#[test]
#[serial(cli_daemon)]
fn cli_pull_alpine_bare_name() {
    if !pull_tests_enabled() {
        eprintln!("skipped: set MINIBOX_E2E_PULL=1 to enable pull tests");
        return;
    }
    let fixture = CliFixture::start();
    let (success, stdout, stderr) = fixture.run(&["pull", "alpine"]);
    assert!(
        success,
        "pull alpine should succeed.\nstdout: {stdout}\nstderr: {stderr}"
    );
}

#[test]
#[serial(cli_daemon)]
fn cli_pull_alpine_with_embedded_tag() {
    if !pull_tests_enabled() {
        return;
    }
    let fixture = CliFixture::start();
    let (success, stdout, stderr) = fixture.run(&["pull", "alpine:latest"]);
    assert!(
        success,
        "pull alpine:latest should succeed (no double-tag).\nstdout: {stdout}\nstderr: {stderr}"
    );
}

#[test]
#[serial(cli_daemon)]
fn cli_pull_with_explicit_tag_flag() {
    if !pull_tests_enabled() {
        return;
    }
    let fixture = CliFixture::start();
    let (success, stdout, stderr) = fixture.run(&["pull", "alpine", "--tag", "3.20"]);
    assert!(
        success,
        "pull alpine --tag 3.20 should succeed.\nstdout: {stdout}\nstderr: {stderr}"
    );
}

#[test]
#[serial(cli_daemon)]
fn cli_pull_with_platform_flag() {
    if !pull_tests_enabled() {
        return;
    }
    let fixture = CliFixture::start();
    let (success, stdout, stderr) = fixture.run(&["pull", "alpine", "--platform", "linux/arm64"]);
    assert!(
        success,
        "pull with --platform should succeed.\nstdout: {stdout}\nstderr: {stderr}"
    );
}

#[test]
#[serial(cli_daemon)]
fn cli_pull_nonexistent_image_fails() {
    if !pull_tests_enabled() {
        return;
    }
    let fixture = CliFixture::start();
    let (success, _stdout, stderr) = fixture.run(&["pull", "nonexistent-image-xyz-99999"]);
    assert!(
        !success,
        "pull of nonexistent image should fail.\nstderr: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Pull idempotency
// ---------------------------------------------------------------------------

#[test]
#[serial(cli_daemon)]
fn cli_pull_twice_succeeds() {
    if !pull_tests_enabled() {
        return;
    }
    let fixture = CliFixture::start();
    let (s1, _, stderr1) = fixture.run(&["pull", "alpine"]);
    assert!(s1, "first pull should succeed.\nstderr: {stderr1}");

    let (s2, _, stderr2) = fixture.run(&["pull", "alpine"]);
    assert!(
        s2,
        "second pull should succeed (idempotent).\nstderr: {stderr2}"
    );
}

// ---------------------------------------------------------------------------
// Prune / rmi on empty store
// ---------------------------------------------------------------------------

#[test]
#[serial(cli_daemon)]
fn cli_prune_empty_store() {
    let fixture = CliFixture::start();
    let (success, stdout, stderr) = fixture.run(&["prune"]);
    assert!(
        success,
        "prune on empty store should succeed.\nstdout: {stdout}\nstderr: {stderr}"
    );
}

#[test]
#[serial(cli_daemon)]
fn cli_rmi_nonexistent_image() {
    let fixture = CliFixture::start();
    // Current daemon returns success for rmi of nonexistent images.
    // Just verify it doesn't crash or hang.
    let (_success, _stdout, _stderr) = fixture.run(&["rmi", "nonexistent:latest"]);
}

// ---------------------------------------------------------------------------
// Stop / rm nonexistent container
// ---------------------------------------------------------------------------

#[test]
#[serial(cli_daemon)]
fn cli_stop_nonexistent_fails() {
    let fixture = CliFixture::start();
    let (success, _stdout, stderr) = fixture.run(&["stop", "nonexistent-container-id"]);
    assert!(
        !success,
        "stop of nonexistent container should fail.\nstderr: {stderr}"
    );
}

#[test]
#[serial(cli_daemon)]
fn cli_rm_nonexistent_fails() {
    let fixture = CliFixture::start();
    let (success, _stdout, stderr) = fixture.run(&["rm", "nonexistent-container-id"]);
    assert!(
        !success,
        "rm of nonexistent container should fail.\nstderr: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Run (cross-platform — adapter may not support exec, so we test error path)
// ---------------------------------------------------------------------------

#[test]
#[serial(cli_daemon)]
fn cli_run_without_pull_fails_gracefully() {
    if !pull_tests_enabled() {
        return;
    }
    let fixture = CliFixture::start();
    let (success, _stdout, stderr) =
        fixture.run(&["run", "not-pulled-image", "--", "/bin/echo", "hello"]);
    assert!(
        !success,
        "run with un-pulled image should fail.\nstderr: {stderr}"
    );
}

#[test]
#[serial(cli_daemon)]
fn cli_run_after_pull() {
    if !pull_tests_enabled() {
        return;
    }
    let fixture = CliFixture::start();

    // Pull first
    let (pull_ok, _, stderr) = fixture.run(&["pull", "alpine"]);
    assert!(pull_ok, "pull should succeed.\nstderr: {stderr}");

    // Run — may fail on macOS (adapter limitation) but should not crash/hang
    let mut child = Command::new(&fixture.cli_bin)
        .env("MINIBOX_SOCKET_PATH", &fixture.socket_path)
        .args(["run", "alpine", "--", "/bin/echo", "hello from minibox"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn minibox run");

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.wait_with_output().expect("wait_with_output");
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                // On Linux with full adapter: should succeed with output
                // On macOS smolvm: may error but should not hang
                eprintln!("run exit={} stdout={} stderr={}", status, stdout, stderr);
                break;
            }
            Ok(None) => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    panic!("minibox run did not exit within 30s");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => panic!("try_wait failed: {e}"),
        }
    }
}
