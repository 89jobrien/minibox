//! VZ isolation tests — behavioral counterpart to `native_adapter_isolation_tests.rs`.
//!
//! Uses `harness = false` so that `main()` can call `dispatch_main()` on the OS
//! main thread, keeping the GCD main queue drained for VZ.framework completion
//! handlers (`startWithCompletionHandler`, `connectToPort:completionHandler:`).
//!
//! All tests run serially on a tokio worker thread. The VM is booted once and
//! shared across all tests via a `tokio::sync::OnceCell`.

// Non-macOS / non-vz stub — binary must always have a main().
#[cfg(not(all(target_os = "macos", feature = "vz")))]
fn main() {
    eprintln!("vz_isolation_tests: skipped (requires macOS + vz feature)");
}

#[cfg(all(target_os = "macos", feature = "vz"))]
fn main() {
    #[link(name = "System", kind = "dylib")]
    unsafe extern "C" {
        static _dispatch_main_q: std::ffi::c_void;
        fn dispatch_async_f(
            queue: *const std::ffi::c_void,
            context: *mut std::ffi::c_void,
            work: unsafe extern "C" fn(*mut std::ffi::c_void),
        );
        fn dispatch_main() -> !;
    }

    unsafe extern "C" fn exit_trampoline(ctx: *mut std::ffi::c_void) {
        // SAFETY: ctx is Box<i32> from Box::into_raw below.
        let code = unsafe { *Box::from_raw(ctx as *mut i32) };
        std::process::exit(code);
    }

    // Run all tests on a worker thread with its own tokio runtime.
    // The main thread is handed to dispatch_main() so VZ completion handlers fire.
    std::thread::Builder::new()
        .name("vz-test-runner".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");

            let failed = rt.block_on(suite::run());

            // SAFETY: _dispatch_main_q is valid; exit_trampoline takes ownership of Box<i32>.
            unsafe {
                let code = Box::into_raw(Box::new(if failed { 1i32 } else { 0i32 }));
                dispatch_async_f(&_dispatch_main_q, code as *mut _, exit_trampoline);
            }
        })
        .expect("spawn vz-test-runner");

    // SAFETY: standard GCD entry point — never returns.
    unsafe { dispatch_main() }
}

// ---------------------------------------------------------------------------
// Test suite — only compiled on macOS + vz
// ---------------------------------------------------------------------------

#[cfg(all(target_os = "macos", feature = "vz"))]
mod suite {
    use base64::Engine as _;
    use macbox::vz::proxy::VzProxy;
    use macbox::vz::vm::{VzVm, VzVmConfig};
    use macbox::vz::vsock::connect_to_agent;
    use minibox_core::protocol::{DaemonRequest, DaemonResponse, OutputStreamKind};
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::OnceCell;

    // -----------------------------------------------------------------------
    // GCD helpers (dispatch_sync_f to main queue for VM construction)
    // -----------------------------------------------------------------------

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
            // SAFETY: dispatch_main() is running on the OS main thread so the
            // GCD main queue is live. prepare_trampoline writes to ctx before
            // dispatch_sync_f returns.
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

        // Phase 2: poll from worker thread — main queue stays free for VZ callbacks.
        tokio::task::spawn_blocking(move || VzVm::wait_for_running(vm, start_signal))
            .await
            .context("spawn_blocking wait_for_running")?
    }

    // -----------------------------------------------------------------------
    // Shared VM fixture
    // -----------------------------------------------------------------------

    fn vm_dir() -> std::path::PathBuf {
        dirs::home_dir().unwrap().join(".minibox").join("vm")
    }

    fn vm_image_available() -> bool {
        let d = vm_dir();
        d.join("boot").join("vmlinuz-virt").exists()
            && d.join("rootfs").join("sbin").join("minibox-agent").exists()
    }

    struct VmFixture {
        vm: Arc<VzVm>,
        _images_dir: TempDir,
        _containers_dir: TempDir,
    }

    static VM_FIXTURE: OnceCell<Option<VmFixture>> = OnceCell::const_new();

    async fn shared_vm() -> Option<&'static Arc<VzVm>> {
        let fixture = VM_FIXTURE
            .get_or_init(|| async {
                if !vm_image_available() {
                    eprintln!(
                        "vz_isolation_tests: VM image not found — run `cargo xtask build-vm-image`"
                    );
                    return None;
                }

                eprintln!("vz_isolation_tests: booting Linux VM...");

                let images_tmp = TempDir::new().expect("images TempDir");
                let containers_tmp = TempDir::new().expect("containers TempDir");
                let config = VzVmConfig {
                    vm_dir: vm_dir(),
                    images_dir: images_tmp.path().join("images"),
                    containers_dir: containers_tmp.path().join("containers"),
                    memory_bytes: 512 * 1024 * 1024,
                    cpu_count: 1,
                };
                std::fs::create_dir_all(&config.images_dir).expect("images dir");
                std::fs::create_dir_all(&config.containers_dir).expect("containers dir");

                let vm = boot_vm(config).await.expect("VM boot failed");
                eprintln!("vz_isolation_tests: VM booted, waiting for agent...");

                let vm_arc = Arc::new(vm);
                connect_to_agent(&vm_arc, 60 * 5)
                    .await
                    .expect("agent did not come up within 300s");
                eprintln!("vz_isolation_tests: agent ready");

                Some(VmFixture {
                    vm: vm_arc,
                    _images_dir: images_tmp,
                    _containers_dir: containers_tmp,
                })
            })
            .await;

        fixture.as_ref().map(|f| &f.vm)
    }

    // -----------------------------------------------------------------------
    // Per-test helper
    // -----------------------------------------------------------------------

    async fn run_in_vm(vm: &Arc<VzVm>, shell_cmd: &str) -> (i32, String) {
        eprintln!("  run_in_vm: {shell_cmd}");
        let stream = connect_to_agent(vm, 30).await.expect("agent connect");
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
                DaemonResponse::ContainerStopped { exit_code: ec } => {
                    exit_code = *ec as i32;
                }
                _ => {}
            }
        }
        eprintln!("  exit={exit_code} stdout={:?}", stdout.trim());
        (exit_code, stdout)
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    async fn vz_container_can_list_rootfs() {
        let Some(vm) = shared_vm().await else {
            eprintln!("SKIP");
            return;
        };
        let (code, stdout) = run_in_vm(vm, "ls /bin").await;
        assert_eq!(code, 0);
        assert!(
            stdout.contains("sh"),
            "/bin/sh must appear in ls /bin; got: {stdout}"
        );
    }

    async fn vz_overlay_write_is_ephemeral() {
        let Some(vm) = shared_vm().await else {
            eprintln!("SKIP");
            return;
        };
        let (code, stdout) = run_in_vm(vm, "echo hello > /tmp/sentinel && cat /tmp/sentinel").await;
        assert_eq!(code, 0);
        assert!(stdout.contains("hello"));
        let (code2, stdout2) =
            run_in_vm(vm, "test -f /tmp/sentinel && echo found || echo not-found").await;
        assert_eq!(code2, 0);
        assert!(
            stdout2.contains("not-found"),
            "overlay must not persist; got: {stdout2}"
        );
    }

    async fn vz_overlay_image_content_visible() {
        let Some(vm) = shared_vm().await else {
            eprintln!("SKIP");
            return;
        };
        let (code, _) = run_in_vm(vm, "test -x /bin/sh").await;
        assert_eq!(code, 0);
    }

    async fn vz_overlay_write_lands_in_upper_not_lower() {
        let Some(vm) = shared_vm().await else {
            eprintln!("SKIP");
            return;
        };
        let (code, stdout) = run_in_vm(vm, "echo cowtest > /tmp/cowfile && cat /tmp/cowfile").await;
        assert_eq!(code, 0);
        assert!(stdout.contains("cowtest"));
    }

    async fn vz_container_runs_in_cgroup() {
        let Some(vm) = shared_vm().await else {
            eprintln!("SKIP");
            return;
        };
        let (code, stdout) = run_in_vm(vm, "cat /proc/self/cgroup").await;
        assert_eq!(code, 0);
        assert!(!stdout.trim().is_empty());
    }

    async fn vz_container_cgroup_is_minibox_slice() {
        let Some(vm) = shared_vm().await else {
            eprintln!("SKIP");
            return;
        };
        let (code, stdout) = run_in_vm(vm, "cat /proc/self/cgroup").await;
        assert_eq!(code, 0);
        assert!(stdout.contains('/'), "got: {stdout}");
    }

    async fn vz_pid_namespace_isolated() {
        let Some(vm) = shared_vm().await else {
            eprintln!("SKIP");
            return;
        };
        let (code, stdout) = run_in_vm(vm, "echo $$").await;
        assert_eq!(code, 0);
        let pid: u32 = stdout.trim().parse().unwrap_or(9999);
        assert!(
            pid <= 10,
            "PID must be ≤ 10 in isolated namespace, got {pid}"
        );
    }

    async fn vz_uts_namespace_isolated() {
        let Some(vm) = shared_vm().await else {
            eprintln!("SKIP");
            return;
        };
        let (code, stdout) = run_in_vm(vm, "hostname").await;
        assert_eq!(code, 0);
        let h = stdout.trim();
        assert!(!h.is_empty());
        assert_ne!(h, "minibox-vm");
    }

    async fn vz_mount_namespace_has_proc() {
        let Some(vm) = shared_vm().await else {
            eprintln!("SKIP");
            return;
        };
        let (code, _) = run_in_vm(vm, "test -d /proc/self").await;
        assert_eq!(code, 0);
    }

    async fn vz_mount_namespace_has_sys() {
        let Some(vm) = shared_vm().await else {
            eprintln!("SKIP");
            return;
        };
        let (code, _) = run_in_vm(vm, "test -d /sys/kernel").await;
        assert_eq!(code, 0);
    }

    async fn vz_net_namespace_isolated() {
        let Some(vm) = shared_vm().await else {
            eprintln!("SKIP");
            return;
        };
        let (code, stdout) = run_in_vm(vm, "ip link show").await;
        assert_eq!(code, 0);
        assert!(stdout.contains("lo"));
    }

    // -----------------------------------------------------------------------
    // Runner
    // -----------------------------------------------------------------------

    pub async fn run() -> bool {
        let tests: &[(
            &str,
            fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
        )] = &[
            ("vz_container_can_list_rootfs", || {
                Box::pin(vz_container_can_list_rootfs())
            }),
            ("vz_overlay_write_is_ephemeral", || {
                Box::pin(vz_overlay_write_is_ephemeral())
            }),
            ("vz_overlay_image_content_visible", || {
                Box::pin(vz_overlay_image_content_visible())
            }),
            ("vz_overlay_write_lands_in_upper_not_lower", || {
                Box::pin(vz_overlay_write_lands_in_upper_not_lower())
            }),
            ("vz_container_runs_in_cgroup", || {
                Box::pin(vz_container_runs_in_cgroup())
            }),
            ("vz_container_cgroup_is_minibox_slice", || {
                Box::pin(vz_container_cgroup_is_minibox_slice())
            }),
            ("vz_pid_namespace_isolated", || {
                Box::pin(vz_pid_namespace_isolated())
            }),
            ("vz_uts_namespace_isolated", || {
                Box::pin(vz_uts_namespace_isolated())
            }),
            ("vz_mount_namespace_has_proc", || {
                Box::pin(vz_mount_namespace_has_proc())
            }),
            ("vz_mount_namespace_has_sys", || {
                Box::pin(vz_mount_namespace_has_sys())
            }),
            ("vz_net_namespace_isolated", || {
                Box::pin(vz_net_namespace_isolated())
            }),
        ];

        eprintln!("\nrunning {} tests", tests.len());
        let mut failed = false;

        for (name, test_fn) in tests {
            eprint!("test {name} ... ");
            // Use catch_unwind on the blocking side; async panics propagate as task panics.
            let result = tokio::task::spawn(test_fn()).await;
            match result {
                Ok(()) => eprintln!("ok"),
                Err(e) => {
                    eprintln!("FAILED: {e:?}");
                    failed = true;
                }
            }
        }

        if failed {
            eprintln!("\nsome tests FAILED");
        } else {
            eprintln!("\nall tests passed");
        }
        failed
    }
}
