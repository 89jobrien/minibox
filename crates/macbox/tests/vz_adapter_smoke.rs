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

    // Boot VM
    let vm = tokio::task::spawn_blocking(move || VzVm::boot(config))
        .await
        .expect("spawn_blocking failed")
        .expect("VM boot failed");
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
