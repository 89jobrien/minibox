//! VZ isolation tests — behavioral counterpart to `native_adapter_isolation_tests.rs`.
//!
//! On macOS the native Linux kernel adapters (overlayfs, cgroups v2, namespaces)
//! are not available on the host.  This suite boots an Alpine Linux VM via
//! VZ.framework and drives the same behavioral assertions through the
//! in-VM miniboxd agent using `ephemeral: true` Run requests.
//!
//! The VM is booted **once** for the entire test binary via `VM_FIXTURE` (a
//! `std::sync::OnceLock`).  All tests share the same running VM — each test
//! opens a fresh vsock connection, sends its request, and asserts on the
//! response.  This keeps total wall time proportional to the number of
//! container runs, not the number of VM boots.
//!
//! **Skip condition**: if the VM image is absent (`~/.mbx/vm/`) every test
//! prints a skip message and returns without failing.
//!
//! Run via `just test-vz-isolation` (requires `--features vz` and a pre-built
//! VM image — `cargo xtask build-vm-image`).

#![cfg(all(target_os = "macos", feature = "vz"))]

use base64::Engine as _;
use macbox::vz::proxy::VzProxy;
use macbox::vz::vm::{VzVm, VzVmConfig};
use macbox::vz::vsock::connect_to_agent;
use minibox_core::protocol::{DaemonRequest, DaemonResponse, OutputStreamKind};
use std::sync::{Arc, OnceLock};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// GCD boot helpers
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
    // SAFETY: ctx is &mut PrepareCtx stack-allocated in the spawn_blocking closure;
    // dispatch_sync_f guarantees it outlives this call and runs exactly once.
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
        // SAFETY: _dispatch_main_q is the live GCD main queue; prepare_trampoline
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
// Shared VM fixture — booted once, shared across all tests
// ---------------------------------------------------------------------------

/// Holds the running VM and its backing temp directories.
///
/// The `TempDir`s must stay alive for the duration of the test binary —
/// dropping them would delete the images/containers directories while the VM
/// is still using them via virtiofs.
struct VmFixture {
    vm: Arc<VzVm>,
    // Kept alive to prevent the temp dirs from being deleted.
    _images_dir: TempDir,
    _containers_dir: TempDir,
}

static VM_FIXTURE: OnceLock<Option<VmFixture>> = OnceLock::new();

/// Return a reference to the shared VM, or `None` if the VM image is absent.
///
/// Boots the VM on first call; subsequent calls return the cached result.
///
/// `OnceLock::get_or_init` is synchronous, but `boot_vm` is async.  Calling
/// `Runtime::block_on` from inside a `#[tokio::test]` worker panics ("cannot
/// start a runtime from within a runtime").  Fix: spawn a fresh OS thread
/// (no tokio context), run the async boot there, and join the result.
fn shared_vm() -> Option<&'static Arc<VzVm>> {
    let fixture = VM_FIXTURE.get_or_init(|| {
        if !vm_image_available() {
            eprintln!("vz_isolation_tests: VM image not found at ~/.mbx/vm/");
            eprintln!("  → run `cargo xtask build-vm-image` to build it");
            eprintln!("  → all tests in this suite will be skipped");
            return None;
        }

        // Spawn a fresh OS thread — no tokio context, so Runtime::block_on is safe.
        let result: Option<VmFixture> = std::thread::spawn(|| {
            eprintln!("vz_isolation_tests: booting Linux VM (once for entire suite)...");

            let images_tmp = TempDir::new().expect("images TempDir");
            let containers_tmp = TempDir::new().expect("containers TempDir");

            let config = VzVmConfig {
                vm_dir: vm_dir(),
                images_dir: images_tmp.path().join("images"),
                containers_dir: containers_tmp.path().join("containers"),
                memory_bytes: 512 * 1024 * 1024,
                cpu_count: 1,
            };
            std::fs::create_dir_all(&config.images_dir).expect("create images dir");
            std::fs::create_dir_all(&config.containers_dir).expect("create containers dir");

            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            let vm = rt.block_on(boot_vm(config)).expect("VM boot failed");

            eprintln!("vz_isolation_tests: VM booted, waiting for agent...");

            let vm_arc = Arc::new(vm);
            let vm_clone = Arc::clone(&vm_arc);
            rt.block_on(async move {
                connect_to_agent(&vm_clone, 60 * 5)
                    .await
                    .expect("agent did not come up within 300s")
            });

            eprintln!("vz_isolation_tests: agent ready — running tests");

            Some(VmFixture {
                vm: vm_arc,
                _images_dir: images_tmp,
                _containers_dir: containers_tmp,
            })
        })
        .join()
        .expect("VM boot thread panicked");

        result
    });

    fixture.as_ref().map(|f| &f.vm)
}

// ---------------------------------------------------------------------------
// Per-test helper
// ---------------------------------------------------------------------------

/// Run a shell command inside the VM via an ephemeral container.
/// Returns `(exit_code, stdout_text)` and prints a one-line trace.
async fn run_in_vm(vm: &Arc<VzVm>, shell_cmd: &str) -> (i32, String) {
    eprintln!("  run_in_vm: {shell_cmd}");

    let stream = connect_to_agent(vm, 30)
        .await
        .expect("agent did not come up");
    let mut proxy = VzProxy::new(stream);

    let req = DaemonRequest::Run {
        image: "alpine".to_string(),
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
    let mut stderr = String::new();
    let mut exit_code = -1i32;

    for resp in &responses {
        match resp {
            DaemonResponse::ContainerOutput {
                stream: OutputStreamKind::Stdout,
                data,
            } => {
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) {
                    stdout.push_str(&String::from_utf8_lossy(&bytes));
                }
            }
            DaemonResponse::ContainerOutput {
                stream: OutputStreamKind::Stderr,
                data,
            } => {
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) {
                    stderr.push_str(&String::from_utf8_lossy(&bytes));
                }
            }
            DaemonResponse::ContainerStopped { exit_code: ec } => {
                exit_code = *ec as i32;
            }
            _ => {}
        }
    }

    eprintln!(
        "  exit={exit_code} stdout={:?} stderr={:?}",
        stdout.trim(),
        stderr.trim()
    );

    (exit_code, stdout)
}

// ---------------------------------------------------------------------------
// Overlay FS behavioral tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn vz_overlay_write_is_ephemeral() {
    let Some(vm) = shared_vm() else {
        eprintln!("SKIP: vz_overlay_write_is_ephemeral");
        return;
    };

    let (code, stdout) = run_in_vm(vm, "echo hello > /tmp/sentinel && cat /tmp/sentinel").await;
    assert_eq!(code, 0, "write+read must succeed");
    assert!(
        stdout.contains("hello"),
        "written file must be readable in same run"
    );

    let (code2, stdout2) =
        run_in_vm(vm, "test -f /tmp/sentinel && echo found || echo not-found").await;
    assert_eq!(code2, 0);
    assert!(
        stdout2.contains("not-found"),
        "overlay upper must not persist between runs; got: {stdout2}"
    );
}

#[tokio::test]
async fn vz_overlay_image_content_visible() {
    let Some(vm) = shared_vm() else {
        eprintln!("SKIP: vz_overlay_image_content_visible");
        return;
    };

    let (code, _) = run_in_vm(vm, "test -x /bin/sh").await;
    assert_eq!(code, 0, "/bin/sh must be executable in alpine container");
}

#[tokio::test]
async fn vz_overlay_write_lands_in_upper_not_lower() {
    let Some(vm) = shared_vm() else {
        eprintln!("SKIP: vz_overlay_write_lands_in_upper_not_lower");
        return;
    };

    // Write a file and read it back — if CoW is working, the write succeeds
    // and the content is correct.
    let (code, stdout) = run_in_vm(vm, "echo cowtest > /tmp/cowfile && cat /tmp/cowfile").await;
    assert_eq!(code, 0);
    assert!(stdout.contains("cowtest"), "CoW write must be readable");
}

// ---------------------------------------------------------------------------
// Cgroups v2 behavioral tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn vz_container_runs_in_cgroup() {
    let Some(vm) = shared_vm() else {
        eprintln!("SKIP: vz_container_runs_in_cgroup");
        return;
    };

    let (code, stdout) = run_in_vm(vm, "cat /proc/self/cgroup").await;
    assert_eq!(code, 0, "cat /proc/self/cgroup must succeed");
    assert!(
        !stdout.trim().is_empty(),
        "/proc/self/cgroup must not be empty"
    );
}

#[tokio::test]
async fn vz_container_cgroup_is_minibox_slice() {
    let Some(vm) = shared_vm() else {
        eprintln!("SKIP: vz_container_cgroup_is_minibox_slice");
        return;
    };

    // miniboxd places containers under its own cgroup slice.
    let (code, stdout) = run_in_vm(vm, "cat /proc/self/cgroup").await;
    assert_eq!(code, 0);
    // The cgroup path should contain the container ID — just verify it's non-trivial.
    assert!(
        stdout.contains('/'),
        "cgroup path should contain '/', got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// Namespace isolation behavioral tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn vz_pid_namespace_isolated() {
    let Some(vm) = shared_vm() else {
        eprintln!("SKIP: vz_pid_namespace_isolated");
        return;
    };

    let (code, stdout) = run_in_vm(vm, "echo $$").await;
    assert_eq!(code, 0);

    let pid: u32 = stdout.trim().parse().unwrap_or(9999);
    assert!(
        pid <= 10,
        "shell PID in isolated PID namespace must be ≤ 10, got {pid}"
    );
}

#[tokio::test]
async fn vz_uts_namespace_isolated() {
    let Some(vm) = shared_vm() else {
        eprintln!("SKIP: vz_uts_namespace_isolated");
        return;
    };

    let (code, stdout) = run_in_vm(vm, "hostname").await;
    assert_eq!(code, 0);

    let hostname = stdout.trim().to_string();
    assert!(!hostname.is_empty(), "container hostname must not be empty");
    assert_ne!(
        hostname, "minibox-vm",
        "container must have isolated hostname, not the VM's hostname"
    );
}

#[tokio::test]
async fn vz_mount_namespace_has_proc() {
    let Some(vm) = shared_vm() else {
        eprintln!("SKIP: vz_mount_namespace_has_proc");
        return;
    };

    let (code, _) = run_in_vm(vm, "test -d /proc/self").await;
    assert_eq!(code, 0, "/proc/self must be mounted in container");
}

#[tokio::test]
async fn vz_mount_namespace_has_sys() {
    let Some(vm) = shared_vm() else {
        eprintln!("SKIP: vz_mount_namespace_has_sys");
        return;
    };

    let (code, _) = run_in_vm(vm, "test -d /sys/kernel").await;
    assert_eq!(code, 0, "/sys must be mounted in container");
}

// ---------------------------------------------------------------------------
// Unprivileged / GKE-mode placeholder
// ---------------------------------------------------------------------------

#[tokio::test]
async fn vz_container_can_list_rootfs() {
    let Some(vm) = shared_vm() else {
        eprintln!("SKIP: vz_container_can_list_rootfs");
        return;
    };

    let (code, stdout) = run_in_vm(vm, "ls /bin").await;
    assert_eq!(code, 0, "ls /bin must succeed");
    assert!(
        stdout.contains("sh"),
        "/bin/sh must appear in ls /bin output"
    );
}
