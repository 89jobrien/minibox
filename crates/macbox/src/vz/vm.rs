//! VZVirtualMachine wrapper — VzVm boot implementation using objc2-virtualization.

/// Configuration for booting the minibox Linux VM.
#[derive(Debug, Clone)]
pub struct VzVmConfig {
    pub vm_dir: std::path::PathBuf,
    pub images_dir: std::path::PathBuf,
    pub containers_dir: std::path::PathBuf,
    pub memory_bytes: u64,
    pub cpu_count: usize,
}

impl VzVmConfig {
    pub fn kernel_path(&self) -> std::path::PathBuf {
        self.vm_dir.join("boot").join("vmlinuz-virt")
    }
    pub fn initramfs_path(&self) -> std::path::PathBuf {
        self.vm_dir.join("boot").join("initramfs-virt")
    }
    pub fn rootfs_path(&self) -> std::path::PathBuf {
        self.vm_dir.join("rootfs")
    }
}

// The real VzVm type is only available on macOS with the vz feature.
#[cfg(all(target_os = "macos", feature = "vz"))]
mod imp {
    use super::VzVmConfig;
    use anyhow::{Context, Result, bail};
    use objc2::AnyThread;
    use objc2::rc::Retained;
    use objc2_foundation::{NSArray, NSFileHandle, NSString, NSURL};
    use objc2_virtualization::{
        VZDirectorySharingDeviceConfiguration, VZFileHandleSerialPortAttachment, VZLinuxBootLoader,
        VZSerialPortConfiguration, VZSharedDirectory, VZSingleDirectoryShare,
        VZSocketDeviceConfiguration, VZVirtioConsoleDeviceSerialPortConfiguration,
        VZVirtioFileSystemDeviceConfiguration, VZVirtioSocketDeviceConfiguration, VZVirtualMachine,
        VZVirtualMachineConfiguration, VZVirtualMachineState,
    };
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// Convert a filesystem path to an NSURL (file URL).
    ///
    /// # Safety
    ///
    /// NSURL creation from a path string is always valid. The returned `Retained<NSURL>`
    /// keeps the object alive for the caller.
    unsafe fn path_to_nsurl(path: &Path) -> Result<Retained<NSURL>> {
        let path_str = path
            .to_str()
            .with_context(|| format!("path is not valid UTF-8: {}", path.display()))?;
        let ns_path = NSString::from_str(path_str);
        // SAFETY: initFileURLWithPath: is a standard Foundation API that accepts any NSString.
        // The outer function is already unsafe so this inner unsafe block is redundant but explicit.
        #[allow(unused_unsafe)]
        let url = unsafe { NSURL::initFileURLWithPath(NSURL::alloc(), &ns_path) };
        Ok(url)
    }

    /// Build the VZLinuxBootLoader from config paths.
    ///
    /// # Safety
    ///
    /// All VZ object constructors used here are standard Objective-C init methods.
    /// Thread safety is handled by the caller (spawn_blocking ensures we're not in async context).
    unsafe fn build_boot_loader(config: &VzVmConfig) -> Result<Retained<VZLinuxBootLoader>> {
        let kernel_url =
            unsafe { path_to_nsurl(&config.kernel_path()) }.context("kernel path to NSURL")?;
        // SAFETY: initWithKernelURL: is a standard init; kernel_url is a valid file URL.
        let boot_loader = unsafe {
            VZLinuxBootLoader::initWithKernelURL(VZLinuxBootLoader::alloc(), &kernel_url)
        };

        let initrd_path = config.initramfs_path();
        if initrd_path.exists() {
            let initrd_url =
                unsafe { path_to_nsurl(&initrd_path) }.context("initrd path to NSURL")?;
            // SAFETY: setInitialRamdiskURL: is a standard setter; initrd_url is a valid file URL.
            unsafe { boot_loader.setInitialRamdiskURL(Some(&initrd_url)) };
        }

        let cmdline = NSString::from_str("console=hvc0 root=/dev/vda rw rootfstype=virtiofs quiet");
        // SAFETY: setCommandLine: is a standard setter.
        unsafe { boot_loader.setCommandLine(&cmdline) };

        Ok(boot_loader)
    }

    /// Build a single virtiofs device configuration.
    ///
    /// # Safety
    ///
    /// All VZ object constructors used here are standard Objective-C init methods.
    unsafe fn build_virtiofs(
        tag: &str,
        host_path: &Path,
        read_only: bool,
    ) -> Result<Retained<VZVirtioFileSystemDeviceConfiguration>> {
        let host_url =
            unsafe { path_to_nsurl(host_path) }.context("virtiofs host path to NSURL")?;
        let ns_tag = NSString::from_str(tag);

        // SAFETY: initWithURL:readOnly: is a standard init; host_url is a valid file URL.
        let shared_dir = unsafe {
            VZSharedDirectory::initWithURL_readOnly(
                VZSharedDirectory::alloc(),
                &host_url,
                read_only,
            )
        };
        // SAFETY: initWithDirectory: is a standard init; shared_dir is a valid VZSharedDirectory.
        let share = unsafe {
            VZSingleDirectoryShare::initWithDirectory(VZSingleDirectoryShare::alloc(), &shared_dir)
        };
        // SAFETY: initWithTag: is a standard init; ns_tag is a valid NSString.
        let fs_dev = unsafe {
            VZVirtioFileSystemDeviceConfiguration::initWithTag(
                VZVirtioFileSystemDeviceConfiguration::alloc(),
                &ns_tag,
            )
        };
        // SAFETY: setShare: is a standard setter; share is a valid VZSingleDirectoryShare which
        // conforms to VZDirectoryShare.
        unsafe {
            fs_dev.setShare(Some(&*share));
        }
        Ok(fs_dev)
    }

    /// Build a serial port configuration wired to the current process's stderr.
    ///
    /// # Safety
    ///
    /// NSFileHandle::fileHandleWithStandardError is a class method that returns a
    /// borrowed reference; we do not own it and must not release it — wrapping in
    /// Retained would be incorrect.  The returned serial port config holds a
    /// reference counted copy internally after setAttachment:.
    unsafe fn build_serial_port() -> Retained<VZVirtioConsoleDeviceSerialPortConfiguration> {
        // SAFETY: VZVirtioConsoleDeviceSerialPortConfiguration::new() follows ObjC +new convention.
        let serial_cfg = unsafe { VZVirtioConsoleDeviceSerialPortConfiguration::new() };
        // SAFETY: fileHandleWithStandardError returns an autoreleased borrow; we only
        // pass it as a reference to setAttachment: which copies/retains internally.
        #[allow(unused_unsafe)]
        let stderr_handle = unsafe { NSFileHandle::fileHandleWithStandardError() };
        // SAFETY: initWithFileHandleForReading:fileHandleForWriting: is a standard init.
        // stderr_handle is a Retained<NSFileHandle>; we borrow it as &NSFileHandle via Deref.
        let attachment = unsafe {
            VZFileHandleSerialPortAttachment::initWithFileHandleForReading_fileHandleForWriting(
                VZFileHandleSerialPortAttachment::alloc(),
                None,
                Some(&*stderr_handle),
            )
        };
        // SAFETY: setAttachment: is a standard setter; attachment is a valid
        // VZFileHandleSerialPortAttachment which conforms to VZSerialPortAttachment.
        unsafe {
            // Upcast via AsMut/Deref chain: VZVirtioConsoleDeviceSerialPortConfiguration
            // inherits setAttachment: from VZSerialPortConfiguration.
            serial_cfg.setAttachment(Some(&*attachment));
        }
        serial_cfg
    }

    /// Handle to a running Virtualization.framework Linux VM.
    pub struct VzVm {
        vm: Retained<VZVirtualMachine>,
        #[allow(dead_code)]
        config: VzVmConfig,
    }

    // SAFETY: VZVirtualMachine is an ObjC object whose retain/release are atomic and
    // thread-safe. All mutable VZ operations must be performed on the queue the VM
    // was created with (main queue in our case); we enforce this by calling boot()
    // from spawn_blocking which may run on any thread, but the VZ main-queue init
    // means callbacks and state queries are serialised by GCD. Storing the
    // Retained<VZVirtualMachine> in a Rust struct and moving it across threads is
    // safe as long as we do not invoke queue-bound methods from non-queue threads
    // after construction — our stop() uses stopWithCompletionHandler: which is
    // documented to be callable from any thread.
    unsafe impl Send for VzVm {}
    // SAFETY: Same reasoning as Send — no interior mutability is exposed from Rust;
    // all mutation goes through ObjC message sends that are internally serialised.
    unsafe impl Sync for VzVm {}

    impl VzVm {
        /// Boot the Linux VM described by `config`.
        ///
        /// This function builds the `VZVirtualMachineConfiguration`, validates it,
        /// creates the `VZVirtualMachine`, starts it asynchronously, and polls until
        /// the VM reaches the `Running` state (or an error/timeout occurs).
        ///
        /// Must be called from a blocking context (e.g. inside `tokio::task::spawn_blocking`).
        pub fn boot(config: VzVmConfig) -> Result<Self> {
            use crate::vz::bindings::load_vz_framework;

            load_vz_framework().context("load Virtualization.framework")?;

            // ------------------------------------------------------------------
            // Build configuration
            // ------------------------------------------------------------------
            // SAFETY: All VZ object allocations below are standard ObjC inits.
            // We are in a blocking context so no async-runtime constraints apply.
            let vm_config = unsafe { VZVirtualMachineConfiguration::new() };

            // Boot loader
            let boot_loader =
                unsafe { build_boot_loader(&config) }.context("build VZLinuxBootLoader")?;
            // SAFETY: setBootLoader: is a standard setter; boot_loader is a valid VZLinuxBootLoader
            // which conforms to VZBootLoader.
            unsafe { vm_config.setBootLoader(Some(&*boot_loader)) };

            // Memory and CPU
            // SAFETY: setMemorySize: / setCPUCount: are standard setters.
            unsafe { vm_config.setMemorySize(config.memory_bytes) };
            unsafe { vm_config.setCPUCount(config.cpu_count) };

            // Virtiofs shares: rootfs (read-write), images (read-only), containers (read-write)
            let rootfs_dev = unsafe { build_virtiofs("mbx-rootfs", &config.rootfs_path(), false) }
                .context("build virtiofs mbx-rootfs")?;
            let images_dev = unsafe { build_virtiofs("mbx-images", &config.images_dir, true) }
                .context("build virtiofs mbx-images")?;
            let containers_dev =
                unsafe { build_virtiofs("mbx-containers", &config.containers_dir, false) }
                    .context("build virtiofs mbx-containers")?;

            // Upcast subtype Retained to parent type before building NSArray, because
            // setDirectorySharingDevices: expects NSArray<VZDirectorySharingDeviceConfiguration>.
            // SAFETY: into_super() is a safe upcast in the ObjC class hierarchy.
            let fs_devices: Retained<NSArray<VZDirectorySharingDeviceConfiguration>> =
                NSArray::from_retained_slice(&[
                    rootfs_dev.into_super(),
                    images_dev.into_super(),
                    containers_dev.into_super(),
                ]);
            // SAFETY: setDirectorySharingDevices: is a standard setter.
            unsafe { vm_config.setDirectorySharingDevices(&fs_devices) };

            // Vsock device — upcast VZVirtioSocketDeviceConfiguration → VZSocketDeviceConfiguration.
            // SAFETY: VZVirtioSocketDeviceConfiguration::new() follows ObjC +new.
            let vsock = unsafe { VZVirtioSocketDeviceConfiguration::new() };
            let socket_devices: Retained<NSArray<VZSocketDeviceConfiguration>> =
                NSArray::from_retained_slice(&[vsock.into_super()]);
            // SAFETY: setSocketDevices: is a standard setter.
            unsafe { vm_config.setSocketDevices(&socket_devices) };

            // Serial port → stderr — upcast to VZSerialPortConfiguration.
            let serial_cfg = unsafe { build_serial_port() };
            let serial_devices: Retained<NSArray<VZSerialPortConfiguration>> =
                NSArray::from_retained_slice(&[serial_cfg.into_super()]);
            // SAFETY: setSerialPorts: is a standard setter.
            unsafe { vm_config.setSerialPorts(&serial_devices) };

            // Validate configuration
            // SAFETY: validateWithError: is a standard validation method.
            unsafe { vm_config.validateWithError() }.map_err(|e| {
                anyhow::anyhow!("VZVirtualMachineConfiguration validation failed: {:?}", e)
            })?;

            // ------------------------------------------------------------------
            // Create VZVirtualMachine (uses main queue)
            // ------------------------------------------------------------------
            // SAFETY: initWithConfiguration: is a standard init; vm_config is a valid,
            // validated VZVirtualMachineConfiguration.
            let vm = unsafe {
                VZVirtualMachine::initWithConfiguration(VZVirtualMachine::alloc(), &vm_config)
            };

            // ------------------------------------------------------------------
            // Start the VM and wait for Running state
            // ------------------------------------------------------------------
            // We use a Mutex<Option<Result<(), String>>> to pass the completion result
            // from the ObjC block back to the Rust polling loop.
            let start_result: Arc<Mutex<Option<Result<(), String>>>> = Arc::new(Mutex::new(None));
            let start_result_clone = Arc::clone(&start_result);

            // SAFETY: startWithCompletionHandler: is a standard async method.
            // The block captures start_result_clone via Arc and is only called once,
            // after which the Arc reference count drops. NSError pointer is either
            // null (success) or a valid autoreleased error object.
            let block = block2::RcBlock::new(move |error: *mut objc2_foundation::NSError| {
                let mut guard = start_result_clone
                    .lock()
                    .expect("start_result mutex poisoned");
                if error.is_null() {
                    *guard = Some(Ok(()));
                } else {
                    // SAFETY: error is non-null and is a valid NSError pointer provided by VZ.
                    let description = unsafe { &*error }.localizedDescription();
                    *guard = Some(Err(description.to_string()));
                }
            });
            // SAFETY: startWithCompletionHandler: accepts a block matching `^(NSError*)`.
            unsafe { vm.startWithCompletionHandler(&*block) };

            // Poll until the completion handler fires or the VM reaches Running/Error state.
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                // Check completion handler result first.
                {
                    let guard = start_result.lock().expect("start_result mutex poisoned");
                    if let Some(ref outcome) = *guard {
                        match outcome {
                            Ok(()) => break,
                            Err(msg) => bail!("VM start failed: {}", msg),
                        }
                    }
                }

                // Also check VM state directly.
                // SAFETY: state is a standard property getter.
                let state = unsafe { vm.state() };
                match state {
                    VZVirtualMachineState::Running => break,
                    VZVirtualMachineState::Error => {
                        bail!("VZVirtualMachine entered Error state during boot")
                    }
                    _ => {}
                }

                if Instant::now() > deadline {
                    bail!("timed out waiting for VZVirtualMachine to reach Running state");
                }

                std::thread::sleep(Duration::from_millis(50));
            }

            tracing::info!(
                memory_bytes = config.memory_bytes,
                cpu_count = config.cpu_count,
                rootfs = %config.rootfs_path().display(),
                "vz: VM started"
            );

            Ok(VzVm { vm, config })
        }

        /// Stop the running VM.
        pub fn stop(&self) {
            // SAFETY: stopWithCompletionHandler: is documented as callable from any thread.
            unsafe {
                self.vm.stopWithCompletionHandler(&*block2::RcBlock::new(
                    |_error: *mut objc2_foundation::NSError| {
                        tracing::debug!("vz: VM stop completion handler called");
                    },
                ));
            }
        }
    }
}

// On non-macOS or without the vz feature, provide a stub that always returns an error.
#[cfg(not(all(target_os = "macos", feature = "vz")))]
mod imp {
    use super::VzVmConfig;
    use anyhow::{Result, bail};

    pub struct VzVm;

    impl VzVm {
        pub fn boot(_config: VzVmConfig) -> Result<Self> {
            bail!("VzVm requires macOS 11+ and the `vz` feature")
        }

        pub fn stop(&self) {}
    }
}

pub use imp::VzVm;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vz_vm_config_fields_are_accessible() {
        let tmp = std::env::temp_dir();
        let cfg = VzVmConfig {
            vm_dir: tmp.clone(),
            images_dir: tmp.clone(),
            containers_dir: tmp.clone(),
            memory_bytes: 512 * 1024 * 1024,
            cpu_count: 2,
        };
        assert_eq!(cfg.cpu_count, 2);
    }

    #[test]
    fn vz_vm_config_paths_use_vm_dir() {
        let vm_dir = std::path::PathBuf::from("/tmp/test-vm");
        let cfg = VzVmConfig {
            vm_dir: vm_dir.clone(),
            images_dir: Default::default(),
            containers_dir: Default::default(),
            memory_bytes: 0,
            cpu_count: 0,
        };
        assert_eq!(cfg.kernel_path(), vm_dir.join("boot").join("vmlinuz-virt"));
        assert_eq!(cfg.rootfs_path(), vm_dir.join("rootfs"));
    }
}
