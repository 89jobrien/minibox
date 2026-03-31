//! Host-side vsock connection to the minibox agent running inside the VZ Linux VM.
//!
//! Apple's Virtualization.framework exposes vsock via `VZVirtioSocketDevice` — a
//! paravirtualised transport that bypasses the network stack entirely.  The host
//! initiates outbound connections with `connectToPort:completionHandler:`, which
//! calls the block with a `VZVirtioSocketConnection` containing a raw file
//! descriptor owned by the ObjC object.
//!
//! We duplicate that fd with `dup(2)` before the connection object is released so
//! that Tokio can take ownership of the fd via `UnixStream::from_raw_fd`.  The
//! socket behaves as a bidirectional byte stream — identical to a Unix socket from
//! Rust/Tokio's perspective.
//!
//! On non-macOS / non-`vz` feature builds the public API is provided by a stub
//! module that returns an error, so the rest of the codebase compiles everywhere.

/// CID assigned to the Linux guest by VZ.framework (always 3 on Apple silicon).
pub const GUEST_CID: u32 = 3;

/// The port that `minibox-agent` listens on inside the VM.
pub const AGENT_PORT: u32 = 9000;

// ---------------------------------------------------------------------------
// Real implementation — macOS + `vz` feature
// ---------------------------------------------------------------------------

#[cfg(all(target_os = "macos", feature = "vz"))]
mod imp {
    use super::AGENT_PORT;
    use crate::vz::vm::VzVm;
    use anyhow::{Context, Result, bail};
    use objc2_virtualization::VZVirtioSocketDevice;
    use std::os::unix::io::RawFd;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tokio::net::UnixStream;

    /// Connect to the `minibox-agent` running at `AGENT_PORT` inside `vm`.
    ///
    /// Retries up to `max_attempts` times with 200 ms back-off between attempts,
    /// to give the guest time to finish booting and start the agent listener.
    ///
    /// Returns a [`tokio::net::UnixStream`] whose underlying fd is the vsock
    /// connection.  Despite the type name the fd is an `AF_VSOCK` socket, but
    /// Tokio treats it as an opaque async I/O source — reads/writes work
    /// correctly.
    pub async fn connect_to_agent(vm: &VzVm, max_attempts: u32) -> Result<UnixStream> {
        let mut last_err = anyhow::anyhow!("no attempts made");

        for attempt in 1..=max_attempts {
            match try_connect(vm).await {
                Ok(stream) => {
                    tracing::info!(port = AGENT_PORT, attempt, "vsock: connected to agent");
                    return Ok(stream);
                }
                Err(e) => {
                    tracing::debug!(
                        port = AGENT_PORT,
                        attempt,
                        error = %e,
                        "vsock: connect attempt failed, retrying"
                    );
                    last_err = e;
                    if attempt < max_attempts {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                }
            }
        }

        Err(last_err).with_context(|| {
            format!(
                "vsock: failed to connect to agent on port {} after {} attempt(s)",
                AGENT_PORT, max_attempts
            )
        })
    }

    /// Single connection attempt via `VZVirtioSocketDevice.connectToPort:completionHandler:`.
    async fn try_connect(vm: &VzVm) -> Result<UnixStream> {
        // Retrieve the VZVirtioSocketDevice from the running VM.
        //
        // SAFETY: socketDevices is a standard property getter on VZVirtualMachine.
        // The returned NSArray is owned by the VM and remains valid for the
        // lifetime of this function.
        let socket_devices = unsafe { vm.vz_vm().socketDevices() };

        anyhow::ensure!(
            socket_devices.count() > 0,
            "vsock: VM has no socket devices"
        );

        // objectAtIndex: returns a Retained<VZSocketDevice>.
        // We configured VZVirtioSocketDeviceConfiguration so the runtime type is
        // VZVirtioSocketDevice.
        let base = socket_devices.objectAtIndex(0);

        // Downcast VZSocketDevice → VZVirtioSocketDevice.
        // objc2's downcast performs an ObjC isKindOfClass check at runtime.
        let virtio_retained = base
            .downcast::<VZVirtioSocketDevice>()
            .map_err(|_| anyhow::anyhow!("vsock: socket device is not a VZVirtioSocketDevice"))?;
        let virtio_dev: &VZVirtioSocketDevice = &*virtio_retained;

        // Send the connect request.  The completion handler fires on the VZ
        // dispatch queue and communicates the result back via a Mutex-wrapped
        // Option so we can poll from the async context.
        let result: Arc<Mutex<Option<Result<RawFd>>>> = Arc::new(Mutex::new(None));
        let result_clone = Arc::clone(&result);

        // SAFETY: connectToPort:completionHandler: is documented as callable from
        // any thread.  The block captures result_clone via Arc and is called
        // exactly once by the VZ runtime.  The VZVirtioSocketConnection pointer
        // passed to the block is either null (error) or a valid autoreleased
        // object that lives for the block's execution duration.
        let block = block2::RcBlock::new(
            move |conn: *mut objc2_virtualization::VZVirtioSocketConnection,
                  error: *mut objc2_foundation::NSError| {
                let mut guard = result_clone.lock().expect("vsock result mutex poisoned");

                if !error.is_null() {
                    // SAFETY: error is non-null and a valid NSError from VZ.
                    let desc = unsafe { &*error }.localizedDescription();
                    *guard = Some(Err(anyhow::anyhow!("VZ connect error: {}", desc)));
                    return;
                }

                if conn.is_null() {
                    *guard = Some(Err(anyhow::anyhow!(
                        "VZ connect returned null connection without error"
                    )));
                    return;
                }

                // SAFETY: conn is non-null and a valid VZVirtioSocketConnection.
                let raw_fd = unsafe { (&*conn).fileDescriptor() };
                if raw_fd < 0 {
                    *guard = Some(Err(anyhow::anyhow!(
                        "VZVirtioSocketConnection returned fd={}, connection closed",
                        raw_fd
                    )));
                    return;
                }

                // Duplicate the fd so Tokio owns a separate copy.  The ObjC
                // object will close its copy when it is deallocated; our dup'd
                // copy remains valid.
                //
                // SAFETY: raw_fd is a valid open file descriptor provided by VZ.
                // dup(2) returns -1 on error.
                let dup_fd = unsafe { libc::dup(raw_fd) };
                if dup_fd < 0 {
                    let err = std::io::Error::last_os_error();
                    *guard = Some(Err(anyhow::anyhow!("dup(vsock fd): {}", err)));
                    return;
                }

                *guard = Some(Ok(dup_fd));
            },
        );

        // SAFETY: connectToPort:completionHandler: accepts a block of type
        // `^(VZVirtioSocketConnection*, NSError*)`.  virtio_dev is a valid
        // VZVirtioSocketDevice obtained from the running VM.
        unsafe { virtio_dev.connectToPort_completionHandler(AGENT_PORT, &*block) };

        // Poll for the result.  VZ fires the completion handler quickly but we
        // must not busy-loop indefinitely.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            {
                let guard = result.lock().expect("vsock result mutex poisoned");
                if let Some(ref outcome) = *guard {
                    let raw_fd = outcome
                        .as_ref()
                        .map(|fd| *fd)
                        .map_err(|e| anyhow::anyhow!("{}", e))?;

                    // Set the fd to non-blocking so Tokio can poll it.
                    //
                    // SAFETY: raw_fd is a valid open file descriptor we own via dup.
                    let flags = unsafe { libc::fcntl(raw_fd, libc::F_GETFL, 0) };
                    if flags < 0 {
                        let err = std::io::Error::last_os_error();
                        // SAFETY: raw_fd is valid; close it to avoid a leak.
                        unsafe { libc::close(raw_fd) };
                        bail!("fcntl(F_GETFL) on vsock fd: {}", err);
                    }
                    // SAFETY: raw_fd is valid; flags is a valid flag set.
                    let rc =
                        unsafe { libc::fcntl(raw_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
                    if rc < 0 {
                        let err = std::io::Error::last_os_error();
                        // SAFETY: raw_fd is valid.
                        unsafe { libc::close(raw_fd) };
                        bail!("fcntl(F_SETFL, O_NONBLOCK) on vsock fd: {}", err);
                    }

                    // SAFETY: raw_fd is a valid non-blocking socket fd that we
                    // own exclusively (obtained via dup above).  We transfer
                    // ownership to std::os::unix::net::UnixStream which will
                    // close it on drop.  Then we wrap with the Tokio async type.
                    let std_stream = unsafe {
                        use std::os::unix::io::FromRawFd;
                        std::os::unix::net::UnixStream::from_raw_fd(raw_fd)
                    };
                    let stream = UnixStream::from_std(std_stream)
                        .context("UnixStream::from_std for vsock fd")?;
                    return Ok(stream);
                }
            }

            if Instant::now() > deadline {
                bail!(
                    "vsock: timed out waiting for VZ completion handler on port {}",
                    AGENT_PORT
                );
            }

            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

// ---------------------------------------------------------------------------
// Stub — non-macOS or without `vz` feature
// ---------------------------------------------------------------------------

#[cfg(not(all(target_os = "macos", feature = "vz")))]
mod imp {
    use crate::vz::vm::VzVm;
    use anyhow::{Result, bail};
    use tokio::net::UnixStream;

    /// Stub: always returns an error on non-macOS / non-vz builds.
    pub async fn connect_to_agent(_vm: &VzVm, _max_attempts: u32) -> Result<UnixStream> {
        bail!("vsock::connect_to_agent requires macOS + the `vz` feature")
    }
}

// Re-export the platform-appropriate function.

/// Connect to the minibox agent inside the VZ Linux VM.
///
/// On macOS with the `vz` feature this establishes a real VZ.framework vsock
/// connection.  On other platforms or without the feature it returns an error.
#[cfg(all(target_os = "macos", feature = "vz"))]
pub use imp::connect_to_agent;

#[cfg(not(all(target_os = "macos", feature = "vz")))]
pub use imp::connect_to_agent;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_addr_constants() {
        assert_eq!(GUEST_CID, 3);
        assert_eq!(AGENT_PORT, 9000);
    }
}
