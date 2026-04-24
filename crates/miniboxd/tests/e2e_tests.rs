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
use helpers::{DaemonFixture, extract_container_id, find_binary, poll_until};
use tempfile::TempDir;

use linuxbox::preflight;
use minibox_core::require_capability;
use serial_test::serial;
use std::process::Command;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Image operation tests
// ---------------------------------------------------------------------------

#[test]
#[serial]
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
#[serial]
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
#[serial]
fn test_e2e_run_echo() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.pull_required("alpine");

    let (success, stdout, _) = fixture.run_cli(&["run", "alpine", "--", "/bin/echo", "hello"]);
    assert!(success, "run should succeed, stdout: {stdout}");
}

#[test]
#[serial]
fn test_e2e_ps_shows_container() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.pull_required("alpine");

    // Spawn a long-lived container in the background; read container ID from
    // the first line emitted by the CLI (ContainerCreated protocol message).
    let (mut cli_child, container_id) =
        fixture.spawn_container_background("alpine", &["/bin/sleep", "30"]);

    // Poll until the container appears in ps (up to 5s)
    let appeared = fixture.wait_for_running(&container_id, Duration::from_secs(5));

    // Stop the container so the CLI child exits cleanly
    let _ = fixture.run_cli(&["stop", &container_id]);
    let _ = cli_child.wait();

    assert!(
        appeared,
        "container {container_id} did not appear as Running in ps within 5s"
    );
}

#[test]
#[serial]
fn test_e2e_stop_container() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.pull_required("alpine");

    let (mut cli_child, container_id) =
        fixture.spawn_container_background("alpine", &["/bin/sleep", "60"]);

    let running = fixture.wait_for_running(&container_id, Duration::from_secs(5));
    assert!(
        running,
        "container {container_id} did not reach Running state within 5s"
    );

    let (success, _, stderr) = fixture.run_cli(&["stop", &container_id]);
    let _ = cli_child.wait();
    assert!(success, "stop should succeed.\nstderr: {stderr}");
}

#[test]
#[serial]
fn test_e2e_rm_container() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.pull_required("alpine");

    let (_, stdout, _) = fixture.run_cli(&["run", "alpine", "--", "/bin/true"]);
    let container_id =
        extract_container_id(&stdout).expect("could not extract container ID from run output");

    // Wait for the container to exit (true exits immediately; poll for Stopped)
    let stopped = poll_until(Duration::from_secs(5), Duration::from_millis(100), || {
        let (ok, ps_out, _) = fixture.run_cli(&["ps"]);
        ok && (!ps_out.contains(&container_id) || ps_out.contains("Stopped"))
    });
    assert!(stopped, "container {container_id} did not stop within 5s");

    let _ = fixture.run_cli(&["stop", &container_id]);

    let (success, _, stderr) = fixture.run_cli(&["rm", &container_id]);
    assert!(success, "rm should succeed.\nstderr: {stderr}");

    let (_, ps_out, _) = fixture.run_cli(&["ps"]);
    assert!(
        !ps_out.contains(&container_id),
        "container should not appear in ps after rm"
    );
}

#[test]
#[serial]
fn test_e2e_rm_running_rejected() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.pull_required("alpine");

    let (mut cli_child, container_id) =
        fixture.spawn_container_background("alpine", &["/bin/sleep", "60"]);

    let running = fixture.wait_for_running(&container_id, Duration::from_secs(5));
    assert!(
        running,
        "container {container_id} did not reach Running state within 5s"
    );

    let (success, _, stderr) = fixture.run_cli(&["rm", &container_id]);
    assert!(
        !success,
        "rm on running container should fail.\nstderr: {stderr}"
    );

    // Clean up: stop the container so the CLI child exits
    let _ = fixture.run_cli(&["stop", &container_id]);
    let _ = cli_child.wait();
}

// ---------------------------------------------------------------------------
// Resource limit tests
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_e2e_run_with_memory_limit() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.pull_required("alpine");

    // Use spawn_run_background so we can check the cgroup while the container
    // is running (run_cli would block until the container exits).
    let (mut cli_child, container_id) = fixture.spawn_run_background(&[
        "alpine",
        "--memory",
        "134217728", // 128MB
        "--",
        "/bin/sleep",
        "30",
    ]);

    let running = fixture.wait_for_running(&container_id, Duration::from_secs(5));
    assert!(
        running,
        "container {container_id} did not reach Running state within 5s"
    );

    let memory_max_path = fixture.cgroup_root.join(&container_id).join("memory.max");
    if memory_max_path.exists() {
        let val = std::fs::read_to_string(&memory_max_path).unwrap_or_default();
        assert_eq!(val.trim(), "134217728", "memory.max should be 128MB");
    }

    let _ = fixture.run_cli(&["stop", &container_id]);
    let _ = cli_child.wait();
}

#[test]
#[serial]
fn test_e2e_run_with_cpu_weight() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.pull_required("alpine");

    let (mut cli_child, container_id) =
        fixture.spawn_run_background(&["alpine", "--cpu-weight", "250", "--", "/bin/sleep", "30"]);

    let running = fixture.wait_for_running(&container_id, Duration::from_secs(5));
    assert!(
        running,
        "container {container_id} did not reach Running state within 5s"
    );

    let cpu_path = fixture.cgroup_root.join(&container_id).join("cpu.weight");
    if cpu_path.exists() {
        let val = std::fs::read_to_string(&cpu_path).unwrap_or_default();
        assert_eq!(val.trim(), "250", "cpu.weight should be 250");
    }

    let _ = fixture.run_cli(&["stop", &container_id]);
    let _ = cli_child.wait();
}

// ---------------------------------------------------------------------------
// Cleanup verification tests
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_e2e_cgroup_cleaned_after_rm() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let fixture = DaemonFixture::start();
    fixture.pull_required("alpine");

    let (_, stdout, _) = fixture.run_cli(&["run", "alpine", "--", "/bin/true"]);
    let container_id =
        extract_container_id(&stdout).expect("could not extract container ID from run output");

    // Poll until stopped
    let stopped = poll_until(Duration::from_secs(5), Duration::from_millis(100), || {
        let (ok, ps_out, _) = fixture.run_cli(&["ps"]);
        ok && (!ps_out.contains(&container_id) || ps_out.contains("Stopped"))
    });
    assert!(stopped, "container {container_id} did not stop within 5s");

    let _ = fixture.run_cli(&["stop", &container_id]);
    let (success, _, stderr) = fixture.run_cli(&["rm", &container_id]);
    assert!(success, "rm should succeed.\nstderr: {stderr}");

    let cgroup_dir = fixture.cgroup_root.join(&container_id);
    assert!(
        !cgroup_dir.exists(),
        "cgroup dir should be removed after rm: {cgroup_dir:?}"
    );
}

// ---------------------------------------------------------------------------
// Overlay cleanup test
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_e2e_overlay_cleaned_after_rm() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");
    require_capability!(caps, overlay_fs, "requires overlay filesystem");

    let fixture = DaemonFixture::start();
    fixture.pull_required("alpine");

    let (_, stdout, _) = fixture.run_cli(&["run", "alpine", "--", "/bin/true"]);
    let container_id =
        extract_container_id(&stdout).expect("could not extract container ID from run output");

    let stopped = poll_until(Duration::from_secs(5), Duration::from_millis(100), || {
        let (ok, ps_out, _) = fixture.run_cli(&["ps"]);
        ok && (!ps_out.contains(&container_id) || ps_out.contains("Stopped"))
    });
    assert!(stopped, "container {container_id} did not stop within 5s");

    let _ = fixture.run_cli(&["stop", &container_id]);
    let (success, _, stderr) = fixture.run_cli(&["rm", &container_id]);
    assert!(success, "rm should succeed.\nstderr: {stderr}");

    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    assert!(
        !mounts.contains(&container_id),
        "no overlay mount should remain for container {container_id} after rm"
    );
}

// ---------------------------------------------------------------------------
// Socket/auth test
// ---------------------------------------------------------------------------

#[test]
#[serial]
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
#[serial]
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
#[serial]
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

// ---------------------------------------------------------------------------
// minibox-in-minibox (DinD) test
// ---------------------------------------------------------------------------

/// Spin a nested miniboxd inside a privileged container and verify that it can
/// pull an image and run a container end-to-end.
///
/// **Why this works without overlay-on-overlay:**  The inner daemon's
/// `MINIBOX_DATA_DIR` is bind-mounted from a host tmpfs-backed TempDir.
/// The inner overlay mounts therefore use host tmpfs as their lowerdir/
/// upperdir/workdir — not the outer container's overlay layer — so the kernel
/// never sees overlay-on-overlay.
///
/// **Requirements:** Linux, root, cgroups v2, overlay_fs, network access
#[test]
#[serial]
fn test_e2e_dind_pull_and_run() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");
    require_capability!(caps, overlay_fs, "requires overlay filesystem");

    let fixture = DaemonFixture::start();
    fixture.pull_required("alpine");

    // Host-side temp dirs bind-mounted into the outer container so the inner
    // daemon stores its state on host tmpfs (avoids overlay-on-overlay).
    let inner_data_dir = TempDir::with_prefix("minibox-dind-data-").expect("create dind data dir");
    let inner_run_dir = TempDir::with_prefix("minibox-dind-run-").expect("create dind run dir");

    // Unique cgroup slice for the inner daemon.
    let inner_cgroup_name = format!("minibox-dind-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let inner_cgroup_path = format!("/sys/fs/cgroup/{inner_cgroup_name}");

    let miniboxd_bin = find_binary("miniboxd");
    let minibox_bin = find_binary("minibox");

    // Volume spec strings — must outlive the run_args slice borrow.
    let v_miniboxd = format!("{}:/usr/local/bin/miniboxd:ro", miniboxd_bin.display());
    let v_minibox = format!("{}:/usr/local/bin/minibox:ro", minibox_bin.display());
    let v_cgroup = "/sys/fs/cgroup:/sys/fs/cgroup".to_string();
    let v_data = format!("{}:/minibox-data", inner_data_dir.path().display());
    let v_run = format!("{}:/minibox-run", inner_run_dir.path().display());

    // Shell script executed inside the outer container (busybox ash).
    // 1. Enable cgroup controllers for the inner daemon's slice.
    // 2. Start inner miniboxd in background.
    // 3. Wait up to 10s for socket.
    // 4. Pull alpine via inner daemon.
    // 5. Run echo via inner daemon and verify output.
    let script = format!(
        r#"set -e
mkdir -p /sys/fs/cgroup/{inner_cgroup_name}
echo '+memory +cpu +pids' > /sys/fs/cgroup/{inner_cgroup_name}/cgroup.subtree_control

MINIBOX_DATA_DIR=/minibox-data \
  MINIBOX_RUN_DIR=/minibox-run \
  MINIBOX_CGROUP_ROOT=/sys/fs/cgroup/{inner_cgroup_name} \
  RUST_LOG=error \
  /usr/local/bin/miniboxd &
DAEMON_PID=$!

i=0
while [ "$i" -lt 100 ] && [ ! -S /minibox-run/miniboxd.sock ]; do
  sleep 0.1; i=$((i+1))
done
[ -S /minibox-run/miniboxd.sock ] || (echo 'inner daemon socket timeout' >&2; kill "$DAEMON_PID"; exit 1)

MINIBOX_SOCKET_PATH=/minibox-run/miniboxd.sock /usr/local/bin/minibox pull alpine >/dev/null

OUT=$(MINIBOX_SOCKET_PATH=/minibox-run/miniboxd.sock /usr/local/bin/minibox run alpine -- /bin/echo hello-from-dind)
echo "$OUT" | grep -q hello-from-dind || (echo "unexpected output: $OUT" >&2; kill "$DAEMON_PID"; exit 2)

kill "$DAEMON_PID" 2>/dev/null || true
wait "$DAEMON_PID" 2>/dev/null || true
echo dind-ok
"#
    );

    let run_args: Vec<&str> = vec![
        "run",
        "--privileged",
        "-v",
        &v_miniboxd,
        "-v",
        &v_minibox,
        "-v",
        &v_cgroup,
        "-v",
        &v_data,
        "-v",
        &v_run,
        "alpine",
        "--",
        "/bin/sh",
        "-c",
        &script,
    ];

    let (exit_code, stdout, stderr) = fixture.run_cli_with_exit_code(&run_args);

    // Best-effort cleanup of the inner cgroup slice on the host.
    let _ = std::fs::remove_dir_all(&inner_cgroup_path);

    assert_eq!(
        exit_code, 0,
        "DinD container exited non-zero.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("dind-ok"),
        "expected 'dind-ok' in DinD output.\nstdout: {stdout}"
    );
}
