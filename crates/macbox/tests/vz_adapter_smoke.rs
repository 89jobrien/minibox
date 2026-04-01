//! Integration smoke test for the VZ adapter.
//!
//! Requires:
//!   - macOS + Apple Silicon (or x86 Mac with VZ.framework)
//!   - VM image at ~/.mbx/vm/ (run `cargo xtask build-vm-image` first)
//!   - `vz` feature compiled in
//!
//! Automatically skipped if VM image is absent.

#![cfg(all(target_os = "macos", feature = "vz"))]

use macbox::vz::proxy::VzProxy;
use macbox::vz::vm::{VzVm, VzVmConfig};
use macbox::vz::vsock::connect_to_agent;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use std::sync::Arc;

fn vm_dir() -> std::path::PathBuf {
    dirs::home_dir().unwrap().join(".mbx").join("vm")
}

fn vm_image_available() -> bool {
    let d = vm_dir();
    d.join("boot").join("vmlinuz-virt").exists()
        && d.join("rootfs").join("sbin").join("minibox-agent").exists()
}

// Two-phase VM boot using the GCD main queue, matching the pattern in macbox/src/lib.rs.
//
// VZ.framework requires VZVirtualMachineConfiguration and VZVirtualMachine to be
// constructed on the GCD main queue. In tests the GCD main queue is available but
// dispatch_main() is not running, so we drive it manually via dispatch_sync_f.
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
    // SAFETY: ctx is a valid &mut PrepareCtx allocated on the stack; dispatch_sync_f
    // guarantees it outlives this call and that this function runs exactly once.
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
        // SAFETY: _dispatch_main_q is the GCD main queue; prepare_trampoline writes
        // to ctx before dispatch_sync_f returns; ctx is stack-allocated and outlives the call.
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

#[tokio::test]
async fn vz_smoke_list_containers_returns_empty() {
    if !vm_image_available() {
        eprintln!("SKIP: VM image not found at ~/.mbx/vm/ — run `cargo xtask build-vm-image`");
        return;
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
    let vm = Arc::new(vm);

    // Wait for agent
    let stream = connect_to_agent(&vm, 60)
        .await
        .expect("agent did not come up");
    let mut proxy = VzProxy::new(stream);

    // Send ListContainers
    let responses = proxy
        .send_request(&DaemonRequest::List)
        .await
        .expect("request failed");

    assert!(!responses.is_empty(), "agent returned no responses");
    let last = responses.last().unwrap();
    assert!(
        matches!(last, DaemonResponse::ContainerList { .. }),
        "expected ContainerList, got {last:?}"
    );

    vm.stop();
}
