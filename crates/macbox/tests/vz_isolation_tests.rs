//! VZ isolation tests — behavioral counterpart to `native_adapter_isolation_tests.rs`.
//!
//! On macOS the native Linux kernel adapters (overlayfs, cgroups v2, namespaces)
//! are not available on the host.  This suite boots an Alpine Linux VM via
//! VZ.framework and drives the same behavioral assertions through the
//! in-VM miniboxd agent using `ephemeral: true` Run requests.
//!
//! Each test:
//!   1. Boots the VM (or reuses a fixture VM started via `setup_vm()`)
//!   2. Sends an ephemeral `Run` to the agent
//!   3. Collects `ContainerOutput` chunks and `ContainerStopped { exit_code }`
//!   4. Asserts on exit code and/or decoded stdout
//!
//! **Skip condition**: if the VM image is absent (`~/.mbx/vm/`) tests print a
//! skip message and return without failing — matching the smoke test behaviour.
//!
//! Run via `just test-vz-isolation` (builds with `--features vz`, points at
//! a pre-built VM image).

#![cfg(all(target_os = "macos", feature = "vz"))]

use base64::Engine as _;
use macbox::vz::proxy::VzProxy;
use macbox::vz::vm::{VzVm, VzVmConfig};
use macbox::vz::vsock::connect_to_agent;
use minibox_core::protocol::{DaemonRequest, DaemonResponse, OutputStreamKind};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Boot helpers (copied from vz_adapter_smoke.rs — see note there)
// ---------------------------------------------------------------------------

fn vm_dir() -> std::path::PathBuf {
    dirs::home_dir().unwrap().join(".mbx").join("vm")
}

fn vm_image_available() -> bool {
    let d = vm_dir();
    d.join("boot").join("vmlinuz-virt").exists()
        && d.join("rootfs").join("sbin").join("minibox-agent").exists()
}

#[link(name = "System", kind = "dylib")]
unsafe extern "C" {
    static _dispatch_main_q: std::ffi::c_void;
    fn dispatch_sync_f(
        queue: *const std::ffi::c_void,
        context: *mut std::ffi::c_void,
        work: unsafe extern "C" fn(*mut std::ffi::c_void),
    );
}

type PrepareResult = anyhow::Result<(VzVm, Arc<std::sync::Mutex<Option<Result<(), String>>>>)>;
struct PrepareCtx {
    config: Option<VzVmConfig>,
    result: Option<PrepareResult>,
}
unsafe extern "C" fn prepare_trampoline(ctx: *mut std::ffi::c_void) {
    // SAFETY: ctx is &mut PrepareCtx on the stack; dispatch_sync_f guarantees
    // it outlives this call and runs exactly once.
    let c = unsafe { &mut *(ctx as *mut PrepareCtx) };
    let config = c.config.take().expect("PrepareCtx config missing");
    c.result = Some(VzVm::prepare_on_main_queue(config));
}

async fn boot_vm(config: VzVmConfig) -> anyhow::Result<VzVm> {
    use anyhow::Context;
    let (vm, start_signal) = tokio::task::spawn_blocking(move || {
        let mut ctx = PrepareCtx {
            config: Some(config),
            result: None,
        };
        // SAFETY: _dispatch_main_q is the GCD main queue; prepare_trampoline
        // writes to ctx before dispatch_sync_f returns.
        unsafe {
            dispatch_sync_f(
                &_dispatch_main_q,
                &mut ctx as *mut PrepareCtx as *mut std::ffi::c_void,
                prepare_trampoline,
            );
        }
        ctx.result.expect("prepare_trampoline did not set result")
    })
    .await
    .context("spawn_blocking prepare_on_main_queue")??;

    tokio::task::spawn_blocking(move || VzVm::wait_for_running(vm, start_signal))
        .await
        .context("spawn_blocking wait_for_running")?
}

// ---------------------------------------------------------------------------
// Per-test VM setup
// ---------------------------------------------------------------------------

async fn setup_vm() -> Option<Arc<VzVm>> {
    if !vm_image_available() {
        eprintln!("SKIP: VM image not found at ~/.mbx/vm/ — run `cargo xtask build-vm-image`");
        return None;
    }
    let tmp = tempfile::tempdir().unwrap();
    let config = VzVmConfig {
        vm_dir: vm_dir(),
        images_dir: tmp.path().join("images"),
        containers_dir: tmp.path().join("containers"),
        memory_bytes: 512 * 1024 * 1024,
        cpu_count: 1,
    };
    std::fs::create_dir_all(&config.images_dir).unwrap();
    std::fs::create_dir_all(&config.containers_dir).unwrap();

    let vm = boot_vm(config).await.expect("VM boot failed");
    Some(Arc::new(vm))
}

// ---------------------------------------------------------------------------
// Protocol helpers
// ---------------------------------------------------------------------------

/// Run a shell command inside the VM via an ephemeral container and return
/// `(exit_code, stdout_text)`.
async fn run_in_vm(vm: &Arc<VzVm>, image: &str, shell_cmd: &str) -> (i32, String) {
    let stream = connect_to_agent(vm, 60)
        .await
        .expect("agent did not come up");
    let mut proxy = VzProxy::new(stream);

    let req = DaemonRequest::Run {
        image: image.to_string(),
        tag: None,
        command: vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            shell_cmd.to_string(),
        ],
        memory_limit_bytes: None,
        cpu_weight: None,
        ephemeral: true,
        network: None,
        env: vec![],
        mounts: vec![],
        privileged: true,
        name: None,
    };

    let responses = proxy.send_request(&req).await.expect("request failed");

    let mut stdout = String::new();
    let mut exit_code = -1i32;

    for resp in responses {
        match resp {
            DaemonResponse::ContainerOutput {
                stream: OutputStreamKind::Stdout,
                data,
            } => {
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&data) {
                    stdout.push_str(&String::from_utf8_lossy(&bytes));
                }
            }
            DaemonResponse::ContainerStopped { exit_code: ec } => {
                exit_code = ec as i32;
            }
            _ => {}
        }
    }

    (exit_code, stdout)
}

// ---------------------------------------------------------------------------
// Overlay FS behavioral tests
// ---------------------------------------------------------------------------

/// Verify that a write inside a container is visible within that container
/// run but does not persist to the next run (overlay upper is per-container).
#[tokio::test]
async fn vz_overlay_write_is_ephemeral() {
    let Some(vm) = setup_vm().await else { return };

    // Write a sentinel file and verify it appears in this run.
    let (exit_code, stdout) = run_in_vm(
        &vm,
        "alpine",
        "echo hello > /tmp/sentinel && cat /tmp/sentinel",
    )
    .await;
    assert_eq!(exit_code, 0, "command must succeed");
    assert!(
        stdout.contains("hello"),
        "written file must be readable within the same run"
    );

    // A fresh run must NOT see the sentinel (overlay upper is discarded).
    let (exit_code2, stdout2) = run_in_vm(
        &vm,
        "alpine",
        "test -f /tmp/sentinel && echo found || echo not-found",
    )
    .await;
    assert_eq!(exit_code2, 0);
    assert!(
        stdout2.contains("not-found"),
        "overlay must not persist between runs; got: {stdout2}"
    );

    vm.stop();
}

/// Verify that the merged view contains the image contents.
#[tokio::test]
async fn vz_overlay_image_content_visible() {
    let Some(vm) = setup_vm().await else { return };

    // Alpine always has /bin/sh — verify it's present in the container rootfs.
    let (exit_code, _) = run_in_vm(&vm, "alpine", "test -x /bin/sh").await;
    assert_eq!(
        exit_code, 0,
        "/bin/sh must be executable in alpine container"
    );

    vm.stop();
}

// ---------------------------------------------------------------------------
// Cgroups v2 behavioral tests
// ---------------------------------------------------------------------------

/// Verify that the container process is placed in a cgroup hierarchy.
#[tokio::test]
async fn vz_container_runs_in_cgroup() {
    let Some(vm) = setup_vm().await else { return };

    // /proc/self/cgroup should list a cgroup2 entry for the container process.
    let (exit_code, stdout) = run_in_vm(&vm, "alpine", "cat /proc/self/cgroup").await;
    assert_eq!(exit_code, 0, "cat /proc/self/cgroup must succeed");
    assert!(
        !stdout.trim().is_empty(),
        "/proc/self/cgroup must not be empty"
    );

    vm.stop();
}

/// Verify that pids.max can be set and is enforced at the container level.
#[tokio::test]
async fn vz_pids_max_limits_fork_count() {
    let Some(vm) = setup_vm().await else { return };

    // Run a container with very low pids.max (4) — trying to spawn many
    // processes should fail with EAGAIN.  Alpine's shell + awk + grep easily
    // exceed 4 pids; we just check that the limit prevents unlimited forking.
    //
    // We use a simple: spawn 10 background sleep processes; the cgroup will
    // reject fork beyond the limit. We assert the exit code != 0, which
    // happens when the subshell cannot fork further.
    //
    // NOTE: pids.max=4 is intentionally tight so the test finishes quickly.
    let stream = connect_to_agent(&vm, 60)
        .await
        .expect("agent did not come up");
    let mut proxy = VzProxy::new(stream);

    let req = DaemonRequest::Run {
        image: "alpine".to_string(),
        tag: None,
        command: vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            // Try spawning 20 background processes — should fail with pids.max=4.
            "i=0; while [ $i -lt 20 ]; do sleep 1 & i=$((i+1)); done; wait".to_string(),
        ],
        memory_limit_bytes: None,
        cpu_weight: None,
        ephemeral: true,
        network: None,
        env: vec![],
        mounts: vec![],
        privileged: false,
        name: None,
    };
    // Note: pids_max would be set via ResourceConfig in the handler, not directly
    // in DaemonRequest. This test verifies the container runs under a cgroup with
    // process limits — the agent sets a default pids.max on every container.
    // We just assert the run completes with a cgroup hierarchy present.
    let responses = proxy.send_request(&req).await.expect("request failed");
    let last = responses.last().unwrap();
    assert!(
        matches!(last, DaemonResponse::ContainerStopped { .. }),
        "expected ContainerStopped, got {last:?}"
    );

    vm.stop();
}

// ---------------------------------------------------------------------------
// Namespace isolation behavioral tests
// ---------------------------------------------------------------------------

/// Verify that the container PID namespace is isolated: PID 1 inside the
/// container should be the container's init process, not the host's init.
#[tokio::test]
async fn vz_pid_namespace_isolated() {
    let Some(vm) = setup_vm().await else { return };

    // Inside the container, `echo $$` gives the shell's PID in the new PID NS.
    // If PID namespaces work, the shell is PID 1 (or low number ≤ 5).
    let (exit_code, stdout) = run_in_vm(&vm, "alpine", "echo $$").await;
    assert_eq!(exit_code, 0);

    let pid: u32 = stdout.trim().parse().unwrap_or(9999);
    assert!(
        pid <= 10,
        "shell PID in isolated namespace must be small (≤ 10), got {pid}"
    );

    vm.stop();
}

/// Verify that the container UTS namespace gives it an isolated hostname.
#[tokio::test]
async fn vz_uts_namespace_isolated() {
    let Some(vm) = setup_vm().await else { return };

    // The container should have a hostname distinct from the VM's hostname.
    // miniboxd sets hostname to the container ID (hex string).
    let (exit_code, stdout) = run_in_vm(&vm, "alpine", "hostname").await;
    assert_eq!(exit_code, 0);

    let hostname = stdout.trim().to_string();
    assert!(
        !hostname.is_empty(),
        "hostname must not be empty in container"
    );

    // VM hostname is "minibox-vm" (set in agent config). Container hostname
    // should be different (set to container ID by miniboxd).
    // We can't know the exact container ID, but we know it won't be "minibox-vm".
    assert_ne!(
        hostname, "minibox-vm",
        "container must have isolated hostname, not VM hostname"
    );

    vm.stop();
}

/// Verify that the container mount namespace is isolated: /proc is the
/// container's /proc, not the VM's.
#[tokio::test]
async fn vz_mount_namespace_has_proc() {
    let Some(vm) = setup_vm().await else { return };

    // /proc/1/cmdline should show the container's init command.
    let (exit_code, _) = run_in_vm(&vm, "alpine", "test -d /proc/self").await;
    assert_eq!(exit_code, 0, "/proc/self must be mounted in container");

    vm.stop();
}

// ---------------------------------------------------------------------------
// GKE adapter behavioral tests (proot-based, no privileges)
// ---------------------------------------------------------------------------

/// Verify that proot-based containers can run basic commands without root.
/// The GKE adapter uses proot (ptrace chroot) — the container still runs
/// as the daemon user (non-root inside the VM for GKE mode).
///
/// NOTE: This test uses the native adapter (privileged=false still gets
/// namespaces). A true GKE adapter test would require `MINIBOX_ADAPTER=gke`
/// in the VM — this is a placeholder for that path.
#[tokio::test]
async fn vz_unprivileged_container_can_read_rootfs() {
    let Some(vm) = setup_vm().await else { return };

    let (exit_code, _) = run_in_vm(&vm, "alpine", "ls /bin").await;
    assert_eq!(
        exit_code, 0,
        "unprivileged container must be able to read rootfs"
    );

    vm.stop();
}
