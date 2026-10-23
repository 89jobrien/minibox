//! VZ boot probe — the Phase 0 experiment from
//! `docs/designs/2026-10-22-vz-efi-boot-design.md`.
//!
//! Answers exactly one question: does `VZVirtualMachine.start` report a `VZError` on this OS
//! build, and does the answer depend on (a) the process carrying
//! `com.apple.security.virtualization` and (b) which boot loader is configured?
//!
//! It deliberately avoids minibox's virtiofs/vsock/rootfs wiring: this is the smallest
//! configuration that reaches `startWithCompletionHandler`, so a failure here cannot be
//! blamed on anything above the boot loader.
//!
//! Usage:
//!
//! ```text
//! vz_boot_probe --loader linux|efi [--work-dir DIR] [--wait-secs N] [--hold-secs N] [--disk PATH]
//! ```
//!
//! `--hold-secs` keeps the probe alive after a successful start so the VM's continued
//! state (and guest/firmware serial output) can be observed. `--disk` attaches a
//! read-only virtio block device to either loader; EFI defaults it to `<work-dir>/esp.img`.
//!
//! `--work-dir` must contain `vmlinuz-virt` for `--loader linux` and a raw disk image
//! `esp.img` for `--loader efi`. The probe creates `vars.fd` there on the first EFI run.
//!
//! Output is a single `PROBE_RESULT` line. Exit code: 0 = VZ accepted the start, 1 = VZ
//! returned an error, 2 = configuration/preflight failure or timeout.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::exit
)]

#[cfg(all(target_os = "macos", feature = "vz-probe"))]
fn main() {
    use anyhow::{Context, Result, anyhow, bail};
    use objc2::AnyThread;
    use objc2::rc::Retained;
    use objc2_foundation::{NSArray, NSFileHandle, NSString, NSURL};
    use objc2_virtualization::{
        VZDiskImageStorageDeviceAttachment, VZEFIBootLoader, VZEFIVariableStore,
        VZEFIVariableStoreInitializationOptions, VZFileHandleSerialPortAttachment,
        VZGenericPlatformConfiguration, VZLinuxBootLoader, VZSerialPortConfiguration,
        VZStorageDeviceConfiguration, VZVirtioBlockDeviceConfiguration,
        VZVirtioConsoleDeviceSerialPortConfiguration, VZVirtualMachine,
        VZVirtualMachineConfiguration, VZVirtualMachineState,
    };
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    #[link(name = "System", kind = "dylib")]
    unsafe extern "C" {
        static _dispatch_main_q: std::ffi::c_void;
        fn dispatch_sync_f(
            queue: *const std::ffi::c_void,
            context: *mut std::ffi::c_void,
            work: unsafe extern "C" fn(*mut std::ffi::c_void),
        );
        fn dispatch_main() -> !;
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Loader {
        Linux,
        Efi,
    }

    struct Args {
        loader: Loader,
        work_dir: PathBuf,
        wait_secs: u64,
        hold_secs: u64,
        disk: Option<PathBuf>,
    }

    impl Args {
        fn parse() -> Result<Self> {
            let mut loader = None;
            let mut work_dir = std::env::temp_dir().join("vz-boot-probe");
            let mut wait_secs = 5u64;
            let mut hold_secs = 0u64;
            let mut disk = None;
            let mut it = std::env::args().skip(1);
            while let Some(arg) = it.next() {
                match arg.as_str() {
                    "--loader" => {
                        loader = Some(match it.next().as_deref() {
                            Some("linux") => Loader::Linux,
                            Some("efi") => Loader::Efi,
                            other => bail!("--loader expects linux|efi, got {other:?}"),
                        });
                    }
                    "--work-dir" => {
                        work_dir = PathBuf::from(
                            it.next()
                                .ok_or_else(|| anyhow!("--work-dir needs a value"))?,
                        );
                    }
                    "--wait-secs" => {
                        wait_secs = it
                            .next()
                            .ok_or_else(|| anyhow!("--wait-secs needs a value"))?
                            .parse()
                            .context("--wait-secs must be an integer")?;
                    }
                    "--hold-secs" => {
                        hold_secs = it
                            .next()
                            .ok_or_else(|| anyhow!("--hold-secs needs a value"))?
                            .parse()
                            .context("--hold-secs must be an integer")?;
                    }
                    "--disk" => {
                        disk = Some(PathBuf::from(
                            it.next().ok_or_else(|| anyhow!("--disk needs a value"))?,
                        ));
                    }
                    other => bail!("unexpected argument {other:?}"),
                }
            }
            Ok(Self {
                loader: loader.ok_or_else(|| anyhow!("--loader linux|efi is required"))?,
                work_dir,
                wait_secs,
                hold_secs,
                disk,
            })
        }
    }

    /// Convert a filesystem path to a file NSURL.
    ///
    /// # Safety
    ///
    /// `initFileURLWithPath:` accepts any `NSString`; the returned URL retains the path.
    unsafe fn path_to_nsurl(path: &Path) -> Result<Retained<NSURL>> {
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow!("path is not valid UTF-8: {}", path.display()))?;
        let ns_path = NSString::from_str(path_str);
        // SAFETY: standard Foundation initializer, valid NSString input.
        Ok(NSURL::initFileURLWithPath(NSURL::alloc(), &ns_path))
    }

    /// Wire the guest serial console to this process's stderr so firmware/GRUB/kernel
    /// output is visible in the probe log.
    ///
    /// # Safety
    ///
    /// Standard `ObjC` initializers; the serial configuration retains the attachment.
    unsafe fn build_serial() -> Retained<VZVirtioConsoleDeviceSerialPortConfiguration> {
        // SAFETY: +new follows the ObjC alloc/init convention.
        let serial = unsafe { VZVirtioConsoleDeviceSerialPortConfiguration::new() };
        // SAFETY: class method returning an autoreleased borrowed handle; only passed as a
        // reference to an initializer that retains it.
        let stderr_handle = NSFileHandle::fileHandleWithStandardError();
        // SAFETY: standard initializer; the attachment retains the handle.
        let attachment = unsafe {
            VZFileHandleSerialPortAttachment::initWithFileHandleForReading_fileHandleForWriting(
                VZFileHandleSerialPortAttachment::alloc(),
                None,
                Some(&*stderr_handle),
            )
        };
        // SAFETY: `setAttachment:` is inherited from VZSerialPortConfiguration.
        unsafe { serial.setAttachment(Some(&*attachment)) };
        serial
    }

    type StartSignal = Arc<Mutex<Option<Result<(), String>>>>;

    struct Prepared {
        vm: Retained<VZVirtualMachine>,
        signal: StartSignal,
    }

    /// Build the smallest possible VM configuration for `loader` and call
    /// `startWithCompletionHandler`, returning without polling.
    ///
    /// # Safety
    ///
    /// Must run on the GCD main queue: VZ requires the configuration object and the
    /// `VZVirtualMachine` to be created there.
    unsafe fn prepare(args: &Args) -> Result<Prepared> {
        // SAFETY: called on the GCD main queue by the caller.
        let vm_config = unsafe { VZVirtualMachineConfiguration::new() };
        // SAFETY: plain setters.
        unsafe {
            vm_config.setMemorySize(512 * 1024 * 1024);
            vm_config.setCPUCount(1);
        }

        match args.loader {
            Loader::Linux => {
                let kernel = args.work_dir.join("vmlinuz-virt");
                if !kernel.exists() {
                    bail!("missing kernel: {}", kernel.display());
                }
                let kernel_url = unsafe { path_to_nsurl(&kernel) }?;
                // SAFETY: standard initializer with a valid file URL.
                let boot_loader = unsafe {
                    VZLinuxBootLoader::initWithKernelURL(VZLinuxBootLoader::alloc(), &kernel_url)
                };
                // SAFETY: standard setter.
                unsafe {
                    boot_loader.setCommandLine(&NSString::from_str("console=hvc0"));
                }
                // SAFETY: VZLinuxBootLoader conforms to VZBootLoader.
                unsafe { vm_config.setBootLoader(Some(&*boot_loader)) };
            }
            Loader::Efi => {
                // VZEFIBootLoader requires a VZGenericPlatformConfiguration.
                // SAFETY: called on the GCD main queue.
                let platform = unsafe { VZGenericPlatformConfiguration::new() };
                // SAFETY: standard setter; platform is a VZPlatformConfiguration.
                unsafe { vm_config.setPlatform(&platform) };

                let vars_path = args.work_dir.join("vars.fd");
                let vars_url = unsafe { path_to_nsurl(&vars_path) }?;
                // SAFETY: either reads an existing variable store or creates one; both are
                // standard initializers on the GCD main queue.
                let store = if vars_path.exists() {
                    unsafe {
                        VZEFIVariableStore::initWithURL(VZEFIVariableStore::alloc(), &vars_url)
                    }
                } else {
                    unsafe {
                        VZEFIVariableStore::initCreatingVariableStoreAtURL_options_error(
                            VZEFIVariableStore::alloc(),
                            &vars_url,
                            VZEFIVariableStoreInitializationOptions::AllowOverwrite,
                        )
                    }
                    .map_err(|e| anyhow!("create EFI variable store: {e:?}"))?
                };

                // SAFETY: +new follows the ObjC convention.
                let boot_loader = unsafe { VZEFIBootLoader::new() };
                // SAFETY: standard setter.
                unsafe { boot_loader.setVariableStore(Some(&store)) };
                // SAFETY: VZEFIBootLoader conforms to VZBootLoader.
                unsafe { vm_config.setBootLoader(Some(&*boot_loader)) };
            }
        }

        // Attach a virtio block device when a disk was supplied (and always for EFI, which
        // needs media to boot from). This is kept out of the per-loader arms so cell B can
        // be run with the identical storage layout as cell D.
        let disk_path = args
            .disk
            .clone()
            .or_else(|| (args.loader == Loader::Efi).then(|| args.work_dir.join("esp.img")));
        if let Some(disk_path) = disk_path {
            if !disk_path.exists() {
                bail!("missing disk image: {}", disk_path.display());
            }
            let disk_url = unsafe { path_to_nsurl(&disk_path) }?;
            // SAFETY: standard initializer; read_only = true.
            let attachment = unsafe {
                VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_error(
                    VZDiskImageStorageDeviceAttachment::alloc(),
                    &disk_url,
                    true,
                )
            }
            .map_err(|e| anyhow!("attach disk image: {e:?}"))?;
            // SAFETY: standard initializer with a valid attachment.
            let block = unsafe {
                VZVirtioBlockDeviceConfiguration::initWithAttachment(
                    VZVirtioBlockDeviceConfiguration::alloc(),
                    &attachment,
                )
            };
            let devices: Retained<NSArray<VZStorageDeviceConfiguration>> =
                NSArray::from_retained_slice(&[block.into_super()]);
            // SAFETY: standard setter.
            unsafe { vm_config.setStorageDevices(&devices) };
        }

        // SAFETY: standard setter; the serial configuration retains its attachment.
        let serial = unsafe { build_serial() };
        let serials: Retained<NSArray<VZSerialPortConfiguration>> =
            NSArray::from_retained_slice(&[serial.into_super()]);
        // SAFETY: standard setter.
        unsafe { vm_config.setSerialPorts(&serials) };

        // SAFETY: standard validation method.
        unsafe { vm_config.validateWithError() }
            .map_err(|e| anyhow!("configuration validation failed: {e:?}"))?;

        // SAFETY: validated configuration; called on the GCD main queue.
        let vm = unsafe {
            VZVirtualMachine::initWithConfiguration(VZVirtualMachine::alloc(), &vm_config)
        };

        let signal: StartSignal = Arc::new(Mutex::new(None));
        let writer = Arc::clone(&signal);
        // SAFETY: completion handler block; VZ calls it exactly once on the main queue.
        let block = block2::RcBlock::new(move |error: *mut objc2_foundation::NSError| {
            let outcome = if error.is_null() {
                Ok(())
            } else {
                // SAFETY: non-null NSError provided by VZ.
                let err = unsafe { &*error };
                Err(format!(
                    "domain={} code={} description={}",
                    err.domain(),
                    err.code(),
                    err.localizedDescription()
                ))
            };
            if let Ok(mut slot) = writer.lock() {
                *slot = Some(outcome);
            }
        });
        // SAFETY: standard async start; returns immediately.
        unsafe { vm.startWithCompletionHandler(&block) };

        Ok(Prepared { vm, signal })
    }

    fn run(args: Args) -> Result<i32> {
        macbox::vz::bindings::load_vz_framework().context("load Virtualization.framework")?;

        struct Ctx {
            loader: Loader,
            work_dir: PathBuf,
            disk: Option<PathBuf>,
            result: Option<Result<Prepared>>,
        }
        unsafe extern "C" fn trampoline(ctx: *mut std::ffi::c_void) {
            // SAFETY: ctx is the stack-allocated Ctx in `run`; dispatch_sync_f returns only
            // after this function has run.
            let ctx = unsafe { &mut *ctx.cast::<Ctx>() };
            let args = Args {
                loader: ctx.loader,
                work_dir: ctx.work_dir.clone(),
                wait_secs: 0,
                hold_secs: 0,
                disk: ctx.disk.clone(),
            };
            // SAFETY: running on the GCD main queue, as VZ requires.
            ctx.result = Some(unsafe { prepare(&args) });
        }

        let mut ctx = Ctx {
            loader: args.loader,
            work_dir: args.work_dir.clone(),
            disk: args.disk.clone(),
            result: None,
        };
        // SAFETY: `_dispatch_main_q` is live because `main` calls `dispatch_main()`;
        // `ctx` outlives the synchronous dispatch.
        unsafe {
            dispatch_sync_f(
                &raw const _dispatch_main_q,
                (&raw mut ctx).cast::<std::ffi::c_void>(),
                trampoline,
            );
        }
        let prepared = ctx
            .result
            .ok_or_else(|| anyhow!("prepare trampoline did not run"))??;

        let started = Instant::now();
        let deadline = started + Duration::from_secs(args.wait_secs);
        let mut start_outcome: Option<Result<(), String>> = None;
        loop {
            {
                let slot = prepared
                    .signal
                    .lock()
                    .map_err(|_| anyhow!("signal mutex poisoned"))?;
                if let Some(outcome) = slot.as_ref() {
                    start_outcome = Some(outcome.clone());
                }
            }

            if start_outcome.is_some() {
                break;
            }

            // SAFETY: state getter; safe from any thread.
            if matches!(unsafe { prepared.vm.state() }, VZVirtualMachineState::Error) {
                println!(
                    "PROBE_RESULT start=ok state=error elapsed_ms={}",
                    started.elapsed().as_millis()
                );
                return Ok(1);
            }

            if Instant::now() >= deadline {
                // SAFETY: state getter; safe from any thread.
                let state = unsafe { prepared.vm.state() };
                println!(
                    "PROBE_RESULT start=timeout state={:?} elapsed_ms={}",
                    state,
                    started.elapsed().as_millis()
                );
                return Ok(2);
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        if let Err(msg) = start_outcome.ok_or_else(|| anyhow!("start outcome missing"))? {
            println!(
                "PROBE_RESULT start=error {msg} elapsed_ms={}",
                started.elapsed().as_millis()
            );
            return Ok(1);
        }

        // Start was accepted. Optionally hold to prove the VM keeps running (and to let
        // firmware/guest output reach the serial port) instead of exiting the instant the
        // completion handler fires.
        let hold_until = started + Duration::from_secs(args.hold_secs);
        let mut last_state = VZVirtualMachineState::Running;
        let mut exited_early = false;
        while Instant::now() < hold_until {
            std::thread::sleep(Duration::from_millis(100));
            // SAFETY: state getter; safe from any thread.
            last_state = unsafe { prepared.vm.state() };
            if matches!(
                last_state,
                VZVirtualMachineState::Stopped | VZVirtualMachineState::Error
            ) {
                exited_early = true;
                break;
            }
        }
        println!(
            "PROBE_RESULT start=ok state={last_state:?} held_ms={} exited_early={exited_early}",
            started.elapsed().as_millis()
        );
        if exited_early { Ok(1) } else { Ok(0) }
    }

    let args = match Args::parse() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("vz_boot_probe: {e:#}");
            std::process::exit(2);
        }
    };
    let loader = match args.loader {
        Loader::Linux => "linux",
        Loader::Efi => "efi",
    };
    println!(
        "PROBE loader={loader} work_dir={} wait_secs={} hold_secs={}",
        args.work_dir.display(),
        args.wait_secs,
        args.hold_secs
    );

    // The main thread must service the GCD main queue; all VZ work happens on a worker.
    std::thread::Builder::new()
        .name("vz-probe".into())
        .spawn(move || {
            let code = match run(args) {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("vz_boot_probe: {e:#}");
                    2
                }
            };
            std::process::exit(code);
        })
        .expect("spawn vz-probe thread");

    // SAFETY: documented GCD process entry point; does not return.
    unsafe { dispatch_main() }
}

#[cfg(not(all(target_os = "macos", feature = "vz-probe")))]
fn main() {
    eprintln!("vz_boot_probe requires macOS and --features vz-probe");
    std::process::exit(2);
}
