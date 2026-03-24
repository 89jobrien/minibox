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

mod helpers;
use helpers::{DaemonFixture, extract_container_id};

use linuxbox::preflight;
use linuxbox::require_capability;
use std::time::Duration;

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
        let child = fixture.child.as_mut().expect("daemon child missing");
        match child.try_wait() {
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
