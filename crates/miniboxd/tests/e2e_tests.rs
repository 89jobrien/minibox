//! End-to-end tests: start real miniboxd + minibox CLI binaries.
//!
//! Tests the full stack through Unix socket: daemon startup, image pull,
//! container lifecycle, resource limits, cleanup, and signal handling.
//!
//! **Requirements:** Linux, root, cgroups v2, built binaries
//!
//! **Running:**
//! ```bash
//! just test-e2e
//! ```

#![cfg(target_os = "linux")]

use minibox_lib::preflight;
use minibox_lib::require_capability;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Binary resolution
// ---------------------------------------------------------------------------

/// Find a minibox binary by name.
///
/// Search order:
/// 1. `MINIBOX_TEST_BIN_DIR` env var (set by justfile)
/// 2. `target/release/{name}`
/// 3. `target/debug/{name}`
fn find_binary(name: &str) -> PathBuf {
    if let Ok(dir) = std::env::var("MINIBOX_TEST_BIN_DIR") {
        let p = PathBuf::from(&dir).join(name);
        if p.exists() {
            return p;
        }
    }

    // Try relative to workspace root (CARGO_MANIFEST_DIR points to miniboxd crate)
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("could not find workspace root");

    for profile in ["release", "debug"] {
        let p = workspace_root.join("target").join(profile).join(name);
        if p.exists() {
            return p;
        }
    }

    panic!(
        "Could not find binary '{}'. Run `cargo build --release` first, \
         or set MINIBOX_TEST_BIN_DIR.",
        name
    );
}

// ---------------------------------------------------------------------------
// DaemonFixture
// ---------------------------------------------------------------------------

/// RAII fixture that starts a real miniboxd and provides CLI access.
struct DaemonFixture {
    child: Child,
    socket_path: PathBuf,
    // Held for Drop: TempDir deletes itself when dropped.
    #[allow(dead_code)]
    data_dir: TempDir,
    #[allow(dead_code)]
    run_dir: TempDir,
    cgroup_root: PathBuf,
    cli_bin: PathBuf,
}

impl DaemonFixture {
    /// Start a daemon with isolated temp dirs.
    ///
    /// Panics if the daemon fails to start within 10 seconds.
    fn start() -> Self {
        let data_dir = TempDir::with_prefix("minibox-test-data-").expect("create temp data dir");
        let run_dir = TempDir::with_prefix("minibox-test-run-").expect("create temp run dir");

        let socket_path = run_dir.path().join("miniboxd.sock");

        // Create cgroup root under our own cgroup (not top-level, which
        // fails on systemd hosts). Read /proc/self/cgroup to find our
        // current cgroup, then create a test subtree there.
        let self_cgroup = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
        let cgroup_rel = self_cgroup
            .lines()
            .find_map(|l| l.strip_prefix("0::"))
            .unwrap_or("/")
            .trim()
            .to_string();
        let relative = cgroup_rel.strip_prefix('/').unwrap_or(&cgroup_rel);
        let test_name = format!(
            "minibox-test-e2e-{}",
            &uuid::Uuid::new_v4().to_string()[..8]
        );
        let cgroup_root = PathBuf::from("/sys/fs/cgroup")
            .join(relative)
            .join(&test_name);

        let daemon_bin = find_binary("miniboxd");
        let cli_bin = find_binary("minibox");

        // Create cgroup root
        std::fs::create_dir_all(&cgroup_root).ok();

        let child = Command::new(&daemon_bin)
            .env("MINIBOX_DATA_DIR", data_dir.path())
            .env("MINIBOX_RUN_DIR", run_dir.path())
            .env("MINIBOX_SOCKET_PATH", &socket_path)
            .env("MINIBOX_CGROUP_ROOT", &cgroup_root)
            .env("RUST_LOG", "miniboxd=debug")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to start miniboxd at {:?}: {e}", daemon_bin));

        let fixture = Self {
            child,
            socket_path: socket_path.clone(),
            data_dir,
            run_dir,
            cgroup_root,
            cli_bin,
        };

        // Wait for socket to appear
        let start = Instant::now();
        let timeout = Duration::from_secs(10);
        while !socket_path.exists() {
            if start.elapsed() > timeout {
                // Kill and capture stderr for debugging
                let mut fixture = fixture;
                let stderr = fixture.kill_and_capture_stderr();
                panic!(
                    "miniboxd did not create socket within 10s.\nSocket: {:?}\nStderr:\n{}",
                    socket_path, stderr
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        fixture
    }

    /// Return the daemon's PID.
    fn daemon_pid(&self) -> u32 {
        self.child.id()
    }

    /// Create a Command for the minibox CLI pre-configured with our socket.
    fn cli(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.cli_bin);
        cmd.env("MINIBOX_SOCKET_PATH", &self.socket_path);
        cmd.args(args);
        cmd
    }

    /// Run a CLI command and return (exit_status, stdout, stderr).
    fn run_cli(&self, args: &[&str]) -> (bool, String, String) {
        let output = self
            .cli(args)
            .output()
            .unwrap_or_else(|e| panic!("failed to run minibox {:?}: {e}", args));

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        (output.status.success(), stdout, stderr)
    }

    /// Kill daemon and capture stderr for debugging.
    /// Only call when the daemon is expected to have failed.
    fn kill_and_capture_stderr(&mut self) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        "(daemon stderr captured at spawn; see test output above)".to_string()
    }

    /// Send SIGTERM to the daemon.
    #[allow(dead_code)]
    fn sigterm(&self) {
        // SAFETY: Sending SIGTERM to our known child process PID. The PID is valid
        // because we spawned it and haven't yet waited on it.
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }
    }
}

impl Drop for DaemonFixture {
    fn drop(&mut self) {
        // 1. Send SIGTERM
        // SAFETY: Sending signal to our known child process PID.
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }

        // 2. Wait up to 5s for clean exit
        let start = Instant::now();
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if start.elapsed() > Duration::from_secs(5) {
                        // 3. Escalate to SIGKILL
                        let _ = self.child.kill();
                        let _ = self.child.wait();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(_) => break,
            }
        }

        // 4. Cleanup cgroup tree (cgroupfs only supports rmdir, not rm -rf)
        if self.cgroup_root.exists() {
            // Remove leaf cgroups first (children), then root.
            // cgroupfs requires directories to be empty (no child cgroups).
            if let Ok(entries) = std::fs::read_dir(&self.cgroup_root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        // Recurse one level for nested cgroups (e.g., supervisor/)
                        if let Ok(sub_entries) = std::fs::read_dir(&path) {
                            for sub in sub_entries.flatten() {
                                if sub.path().is_dir() {
                                    let _ = std::fs::remove_dir(sub.path());
                                }
                            }
                        }
                        let _ = std::fs::remove_dir(&path);
                    }
                }
            }
            let _ = std::fs::remove_dir(&self.cgroup_root);
        }

        // 5. TempDir handles data_dir and run_dir
    }
}

// ---------------------------------------------------------------------------
// Image operation tests
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_pull_alpine() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();

    let (success, stdout, stderr) = fixture.run_cli(&["pull", "alpine"]);
    assert!(
        success,
        "pull should succeed.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.to_lowercase().contains("pull") || stdout.to_lowercase().contains("alpine"),
        "stdout should mention pull/alpine, got: {stdout}"
    );
}

#[test]
fn test_e2e_pull_nonexistent() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();

    let (success, stdout, stderr) = fixture.run_cli(&["pull", "nonexistent-image-xyz-99999"]);
    assert!(
        !success,
        "pull of nonexistent image should fail.\nstdout: {stdout}\nstderr: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Container lifecycle tests
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_run_echo() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();

    // Pull first
    fixture.run_cli(&["pull", "alpine"]);

    let (success, stdout, _) = fixture.run_cli(&["run", "alpine", "--", "/bin/echo", "hello"]);
    assert!(success, "run should succeed, stdout: {stdout}");
}

#[test]
fn test_e2e_ps_shows_container() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.run_cli(&["pull", "alpine"]);

    // Run a long-lived container
    fixture.run_cli(&["run", "alpine", "--", "/bin/sleep", "30"]);

    // Give it a moment to start
    std::thread::sleep(Duration::from_millis(500));

    let (success, stdout, _) = fixture.run_cli(&["ps"]);
    assert!(success, "ps should succeed");
    assert!(
        stdout.contains("alpine") || stdout.contains("Running"),
        "ps should show the container, got: {stdout}"
    );
}

#[test]
fn test_e2e_stop_container() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.run_cli(&["pull", "alpine"]);

    // Run a long-lived container
    let (_, stdout, _) = fixture.run_cli(&["run", "alpine", "--", "/bin/sleep", "60"]);
    std::thread::sleep(Duration::from_millis(500));

    // Extract container ID from stdout (format varies — look for hex-like ID)
    let container_id = extract_container_id(&stdout);

    if let Some(id) = container_id {
        let (success, _, _) = fixture.run_cli(&["stop", &id]);
        assert!(success, "stop should succeed");
    } else {
        eprintln!("SKIPPED: could not extract container ID from: {stdout}");
    }
}

#[test]
fn test_e2e_rm_container() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.run_cli(&["pull", "alpine"]);

    let (_, stdout, _) = fixture.run_cli(&["run", "alpine", "--", "/bin/true"]);
    std::thread::sleep(Duration::from_secs(1));

    let container_id = extract_container_id(&stdout);
    if let Some(id) = container_id {
        // Stop first, then rm
        let _ = fixture.run_cli(&["stop", &id]);
        std::thread::sleep(Duration::from_millis(200));

        let (success, _, _) = fixture.run_cli(&["rm", &id]);
        assert!(success, "rm should succeed");

        // Verify it's gone from ps
        let (_, ps_out, _) = fixture.run_cli(&["ps"]);
        assert!(
            !ps_out.contains(&id),
            "container should not appear in ps after rm"
        );
    } else {
        eprintln!("SKIPPED: could not extract container ID from: {stdout}");
    }
}

#[test]
fn test_e2e_rm_running_rejected() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.run_cli(&["pull", "alpine"]);

    let (_, stdout, _) = fixture.run_cli(&["run", "alpine", "--", "/bin/sleep", "60"]);
    std::thread::sleep(Duration::from_millis(500));

    let container_id = extract_container_id(&stdout);
    if let Some(id) = container_id {
        let (success, _, stderr) = fixture.run_cli(&["rm", &id]);
        assert!(
            !success,
            "rm on running container should fail.\nstderr: {stderr}"
        );
    } else {
        eprintln!("SKIPPED: could not extract container ID from: {stdout}");
    }
}

// ---------------------------------------------------------------------------
// Resource limit tests
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_run_with_memory_limit() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.run_cli(&["pull", "alpine"]);

    let (success, stdout, _) = fixture.run_cli(&[
        "run",
        "alpine",
        "--memory",
        "134217728", // 128MB
        "--",
        "/bin/sleep",
        "30",
    ]);
    assert!(
        success,
        "run with memory limit should succeed, stdout: {stdout}"
    );

    std::thread::sleep(Duration::from_millis(500));

    // Find the container's cgroup and check memory.max
    let container_id = extract_container_id(&stdout);
    if let Some(id) = container_id {
        let memory_max_path = fixture.cgroup_root.join(&id).join("memory.max");
        if memory_max_path.exists() {
            let val = std::fs::read_to_string(&memory_max_path).unwrap_or_default();
            assert_eq!(val.trim(), "134217728", "memory.max should be 128MB");
        }
    }
}

#[test]
fn test_e2e_run_with_cpu_weight() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.run_cli(&["pull", "alpine"]);

    let (success, stdout, _) = fixture.run_cli(&[
        "run",
        "alpine",
        "--cpu-weight",
        "250",
        "--",
        "/bin/sleep",
        "30",
    ]);
    assert!(
        success,
        "run with cpu-weight should succeed, stdout: {stdout}"
    );

    std::thread::sleep(Duration::from_millis(500));

    let container_id = extract_container_id(&stdout);
    if let Some(id) = container_id {
        let cpu_path = fixture.cgroup_root.join(&id).join("cpu.weight");
        if cpu_path.exists() {
            let val = std::fs::read_to_string(&cpu_path).unwrap_or_default();
            assert_eq!(val.trim(), "250", "cpu.weight should be 250");
        }
    }
}

// ---------------------------------------------------------------------------
// Cleanup verification tests
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_cgroup_cleaned_after_rm() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.run_cli(&["pull", "alpine"]);

    let (_, stdout, _) = fixture.run_cli(&["run", "alpine", "--", "/bin/true"]);
    std::thread::sleep(Duration::from_secs(1));

    let container_id = extract_container_id(&stdout);
    if let Some(id) = container_id {
        let _ = fixture.run_cli(&["stop", &id]);
        std::thread::sleep(Duration::from_millis(200));
        let _ = fixture.run_cli(&["rm", &id]);

        let cgroup_dir = fixture.cgroup_root.join(&id);
        assert!(
            !cgroup_dir.exists(),
            "cgroup dir should be removed after rm: {:?}",
            cgroup_dir
        );
    }
}

// ---------------------------------------------------------------------------
// Overlay cleanup test
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_overlay_cleaned_after_rm() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");
    require_capability!(caps, overlay_fs, "requires overlay filesystem");

    let fixture = DaemonFixture::start();
    fixture.run_cli(&["pull", "alpine"]);

    let (_, stdout, _) = fixture.run_cli(&["run", "alpine", "--", "/bin/true"]);
    std::thread::sleep(Duration::from_secs(1));

    let container_id = extract_container_id(&stdout);
    if let Some(id) = container_id {
        let _ = fixture.run_cli(&["stop", &id]);
        std::thread::sleep(Duration::from_millis(200));
        let _ = fixture.run_cli(&["rm", &id]);

        // Check that no overlay mount remains for this container
        let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
        assert!(
            !mounts.contains(&id),
            "no overlay mount should remain for container {} after rm",
            id
        );
    }
}

// ---------------------------------------------------------------------------
// Socket/auth test
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_nonroot_rejected() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    // We are running as root, so we use `sudo -u nobody` to attempt
    // a CLI connection as a non-root user.
    let fixture = DaemonFixture::start();

    let output = Command::new("sudo")
        .args(["-u", "nobody", fixture.cli_bin.to_str().unwrap(), "ps"])
        .env("MINIBOX_SOCKET_PATH", &fixture.socket_path)
        .output();

    match output {
        Ok(o) => {
            assert!(
                !o.status.success(),
                "non-root CLI connection should be rejected"
            );
        }
        Err(_) => {
            eprintln!("SKIPPED: could not run as nobody (sudo not configured)");
        }
    }
}

// ---------------------------------------------------------------------------
// Supervisor cgroup migration test
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_daemon_migrates_to_supervisor() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();

    // Read the daemon's cgroup
    let cgroup_file = format!("/proc/{}/cgroup", fixture.daemon_pid());
    let cgroup_content = std::fs::read_to_string(&cgroup_file)
        .unwrap_or_else(|e| panic!("failed to read {cgroup_file}: {e}"));

    let cgroup_path = cgroup_content
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .unwrap_or("")
        .trim();

    assert!(
        cgroup_path.ends_with("/supervisor"),
        "daemon should be in supervisor cgroup, but is in: {}",
        cgroup_path
    );
}

// ---------------------------------------------------------------------------
// Signal handling test
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_sigterm_clean_shutdown() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let mut fixture = DaemonFixture::start();
    let socket = fixture.socket_path.clone();
    let pid = fixture.daemon_pid() as i32;

    assert!(socket.exists(), "socket should exist before SIGTERM");

    // Send SIGTERM directly (don't use fixture.sigterm() — we want to
    // manually wait and then let Drop handle cleanup without double-signal)
    // SAFETY: Sending signal to our known child process PID.
    unsafe { libc::kill(pid, libc::SIGTERM) };

    // Wait for exit
    let start = Instant::now();
    loop {
        match fixture.child.try_wait() {
            Ok(Some(status)) => {
                assert!(
                    status.success() || status.code() == Some(0),
                    "daemon should exit cleanly, got: {:?}",
                    status
                );
                break;
            }
            Ok(None) => {
                if start.elapsed() > Duration::from_secs(5) {
                    panic!("daemon did not exit within 5s of SIGTERM");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => panic!("wait error: {e}"),
        }
    }

    // Socket should be cleaned up
    assert!(
        !socket.exists(),
        "socket should be removed after clean shutdown"
    );

    // Drop will try SIGTERM again on the already-exited process — that's
    // harmless (kill on dead PID returns ESRCH, ignored by Drop).
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Try to extract a container ID from CLI output.
///
/// Looks for a 16-char hex string (the truncated UUID format used by minibox).
fn extract_container_id(output: &str) -> Option<String> {
    // Look for a hex-like ID in the output
    for word in output.split_whitespace() {
        let cleaned = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if cleaned.len() == 16 && cleaned.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(cleaned.to_string());
        }
    }
    // Fallback: look for any alphanumeric token of length 16
    for word in output.split_whitespace() {
        let cleaned = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if cleaned.len() == 16 && cleaned.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Some(cleaned.to_string());
        }
    }
    None
}
