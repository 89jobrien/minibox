#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::allow_attributes_without_reason
)]
//! Tier 3 Linux-only lifecycle **failure-path** tests (#74).
//!
//! Promotes the mock-level scenarios from `adapter_failure_injection_tests.rs`
//! and `lifecycle_error_path_tests.rs` to real kernel infrastructure: real
//! cgroup v2 writes, real overlay mounts, real signals. Where the mock tests
//! assert call ordering, these assert observable kernel state — cgroup
//! directories actually removed, mounts actually gone from `/proc/mounts`,
//! processes actually killed.
//!
//! **Linux only** — gated on `cfg(target_os = "linux")`; individual tests
//! skip gracefully via `require_capability!` when the host lacks root or the
//! required kernel features.
//!
//! Run via `just test-integration` (Linux + root + cgroup v2).

#![cfg(target_os = "linux")]

use minibox::container::cgroups::{CgroupConfig, CgroupManager, cgroup_path_for};
use minibox::container::filesystem::cleanup_bind_mounts;
use minibox::container::{Container, NativeContainerState};
use minibox::preflight::probe;
use minibox_core::domain::BindMount;
use minibox_macros::require_capability;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// True if `path` appears as a mount target in /proc/mounts.
fn is_mounted(path: &Path) -> bool {
    let mounts = fs::read_to_string("/proc/mounts").expect("read /proc/mounts");
    let needle = path.to_string_lossy();
    mounts.lines().any(|l| {
        l.split_whitespace()
            .nth(1)
            .is_some_and(|target| target == needle)
    })
}

/// Spawn a host child process running `script` under `sh -c`.
fn spawn_sh(script: &str) -> std::process::Child {
    std::process::Command::new("sh")
        .args(["-c", script])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn sh")
}

// ---------------------------------------------------------------------------
// Container::start rollback (the step-1/step-2 unwind added for #74)
// ---------------------------------------------------------------------------

/// `Container::start` creates the cgroup (step 1) before mounting the overlay
/// (step 2). If overlay setup fails, the cgroup must be unwound — a leaked
/// cgroup directory survives until reboot and `nuke-test-state` exists
/// precisely because these used to accumulate.
#[test]
fn container_start_overlay_failure_rolls_back_cgroup() {
    let caps = probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let tmp = TempDir::new().expect("unwrap in test");
    let base_dir = tmp.path().join("container");
    fs::create_dir_all(&base_dir).expect("unwrap in test");

    let mut container = Container::new(
        "test-image",
        vec!["/bin/sh".to_string()],
        &base_dir,
        CgroupConfig::default(),
    )
    .expect("Container::new");
    let cgroup_dir = cgroup_path_for(&container.id);

    // Empty layer list makes setup_overlay fail after the cgroup was created.
    let result = container.start(&base_dir, &[], CgroupConfig::default());

    assert!(result.is_err(), "start with no layers must fail");
    assert_eq!(
        container.state,
        NativeContainerState::Created,
        "failed start must not transition state"
    );
    assert!(
        !cgroup_dir.exists(),
        "cgroup dir {} must be rolled back after overlay failure",
        cgroup_dir.display()
    );
}

// ---------------------------------------------------------------------------
// Container::stop signal escalation
// ---------------------------------------------------------------------------

/// A process that traps SIGTERM must be SIGKILLed by `stop()` after the
/// graceful-shutdown window, and the container must still land in Stopped.
#[test]
fn stop_escalates_to_sigkill_when_sigterm_ignored() {
    let tmp = TempDir::new().expect("unwrap in test");
    let base_dir = tmp.path().join("container");
    fs::create_dir_all(&base_dir).expect("unwrap in test");

    // The child signals readiness via a sentinel file only after its TERM
    // trap is installed — without this, stop()'s SIGTERM can race sh startup
    // and win, exercising the graceful path instead of the escalation path.
    let sentinel = tmp.path().join("trap-ready");
    let child = spawn_sh(&format!(
        "trap '' TERM; : > {}; sleep 300",
        sentinel.display()
    ));
    let pid = child.id();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !sentinel.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "child never signalled trap readiness"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let mut container = Container::new(
        "test-image",
        vec!["sleep".to_string()],
        &base_dir,
        CgroupConfig::default(),
    )
    .expect("Container::new");
    container.pid = Some(pid);
    container.state = NativeContainerState::Running;

    container.stop().expect("stop must succeed via SIGKILL");

    assert_eq!(container.state, NativeContainerState::Stopped);
    // After SIGKILL + reap, signalling the PID must fail (process gone).
    let alive = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok();
    assert!(!alive, "PID {pid} must be dead after SIGKILL escalation");
}

/// `stop()` on a container whose process was already reaped must tolerate the
/// ECHILD branch and still transition to Stopped rather than erroring.
#[test]
fn stop_tolerates_already_reaped_pid() {
    let tmp = TempDir::new().expect("unwrap in test");
    let base_dir = tmp.path().join("container");
    fs::create_dir_all(&base_dir).expect("unwrap in test");

    let mut child = spawn_sh("true");
    let pid = child.id();
    child.wait().expect("reap child");

    let mut container = Container::new(
        "test-image",
        vec!["true".to_string()],
        &base_dir,
        CgroupConfig::default(),
    )
    .expect("Container::new");
    container.pid = Some(pid);
    container.state = NativeContainerState::Running;

    container
        .stop()
        .expect("stop must tolerate an already-reaped PID");
    assert_eq!(container.state, NativeContainerState::Stopped);
}

// ---------------------------------------------------------------------------
// Pause/resume against a real cgroup.freeze
// ---------------------------------------------------------------------------

/// `CgroupManager::pause`/`resume` must toggle the real `cgroup.freeze` file
/// for a cgroup holding a live process. This is the first real-freezer
/// coverage — the daemon-level pause/resume tests use mock state only.
#[tokio::test]
async fn pause_resume_toggles_real_cgroup_freeze() {
    let caps = probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let id = format!("lifecycle-freeze-{}", std::process::id());
    let manager = CgroupManager::new(&id, CgroupConfig::default());
    manager.create().expect("cgroup create");

    let mut child = spawn_sh("sleep 300");
    let pid = child.id();
    manager.add_process(pid).expect("add process to cgroup");

    manager.pause().await.expect("pause");
    let frozen = fs::read_to_string(manager.cgroup_path().join("cgroup.freeze"))
        .expect("read cgroup.freeze");
    assert_eq!(frozen.trim(), "1", "cgroup.freeze must be 1 after pause");

    manager.resume().await.expect("resume");
    let thawed = fs::read_to_string(manager.cgroup_path().join("cgroup.freeze"))
        .expect("read cgroup.freeze");
    assert_eq!(thawed.trim(), "0", "cgroup.freeze must be 0 after resume");

    // Kill + reap the process before cleanup so rmdir doesn't hit EBUSY.
    let _ = nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid as i32),
        nix::sys::signal::Signal::SIGKILL,
    );
    let _ = child.wait();
    manager.cleanup().expect("cgroup cleanup");
    assert!(
        !manager.cgroup_path().exists(),
        "cgroup dir must be removed after cleanup"
    );
}

/// `pause()` against a cgroup that was never created must surface an error,
/// not silently succeed — the daemon relies on this to reject pause requests
/// for containers with missing cgroup state.
#[tokio::test]
async fn pause_on_missing_cgroup_errors() {
    let caps = probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");

    let id = format!("lifecycle-missing-{}", std::process::id());
    let manager = CgroupManager::new(&id, CgroupConfig::default());
    // No create() — cgroup.freeze does not exist.
    assert!(
        manager.pause().await.is_err(),
        "pause on a nonexistent cgroup must error"
    );
}

// ---------------------------------------------------------------------------
// Bind-mount teardown
// ---------------------------------------------------------------------------

/// `cleanup_bind_mounts` must actually unmount a real bind mount, and calling
/// it a second time on the already-unmounted target must be a tolerated
/// no-op (best-effort semantics), not a panic or state corruption.
#[test]
fn cleanup_bind_mounts_unmounts_and_is_idempotent() {
    let caps = probe();
    require_capability!(caps, is_root, "requires root");

    let tmp = TempDir::new().expect("unwrap in test");
    let host_dir = tmp.path().join("host-data");
    fs::create_dir_all(&host_dir).expect("unwrap in test");
    fs::write(host_dir.join("marker"), b"host").expect("unwrap in test");

    let rootfs = tmp.path().join("rootfs");
    let target = rootfs.join("data");
    fs::create_dir_all(&target).expect("unwrap in test");

    nix::mount::mount(
        Some(host_dir.as_path()),
        target.as_path(),
        None::<&str>,
        nix::mount::MsFlags::MS_BIND,
        None::<&str>,
    )
    .expect("bind mount");
    assert!(
        is_mounted(&target),
        "bind mount must appear in /proc/mounts"
    );
    assert!(
        target.join("marker").exists(),
        "host content visible through bind mount"
    );

    let mounts = vec![BindMount {
        host_path: host_dir.clone(),
        container_path: std::path::PathBuf::from("/data"),
        read_only: false,
    }];

    cleanup_bind_mounts(&mounts, &rootfs);
    assert!(
        !is_mounted(&target),
        "bind mount must be gone after cleanup_bind_mounts"
    );
    assert!(
        !target.join("marker").exists(),
        "host content must no longer be visible after unmount"
    );

    // Second call on the already-unmounted target: best-effort no-op.
    cleanup_bind_mounts(&mounts, &rootfs);
    assert!(
        rootfs.join("data").exists(),
        "mount point directory itself must survive cleanup"
    );
}
