# Design: VZ EFI Boot Path

**Status**: implemented. Phase 0 gate has been **executed and passed** (cell D; see
"Result" below), which selected Phase 1 as the implementation path. Phase 1 (T1-T5) has
landed and the adapter boots a guest through `VZEFIBootLoader` on macOS 26.x. Updated
2026-11-06 to record the as-built state and the docs/maturity follow-ups (T6).

**Supersedes**: the closing "or minibox switches to `VZEFIBootLoader` ... out of scope for
this design" note in `docs/designs/2026-08-15-vz-adapter-revival-design.md`.

## Problem

The `vz` adapter compiles, passes clippy, and has a working GCD/vsock harness, but cannot
boot a guest on this machine's macOS 26.4:

```text
VM start FAILED: Internal Virtualization error. The virtual machine failed to start.
(domain=VZErrorDomain code=1)
```

The 2026-08-15 design concluded `VZLinuxBootLoader` is broken on macOS 26.x after ruling out
a stale kernel image and a missing `VZGenericPlatformConfiguration`. Those two are genuinely
ruled out; the remaining confound (signing, versus the boot mechanism itself) is what Phase 0
controls explicitly below.

## As-built status (2026-11-19)

Phase 1 landed and is recorded in the capability matrix as `experimental` (docs/core/FEATURE_MATRIX.mbx.md, updated via `cargo xtask docs sync-adapters` from `xtask/context.toml`). Summary of what shipped and what remains unverified:

- `cargo xtask build-vm-image` (xtask/src/vm_image.rs) pins two artifacts by release and sha256 —
  the Alpine virt ISO 3.22.7 (`alpine-virt-3.22.7-aarch64.iso`, sha256 `3e37f238f6317ef56a12f4cff5c4265f99d25785945f777b19d41b35a7fabbc7`) and the
  `linux-virt` APK 6.12.114-r0 (sha256 `345b2d561443cd8c978cbcc0f36f2374b3ef702debb747221384b4ddc07d6a77` for the `/lib/modules` tree). It extracts
  `vmlinuz-virt`, `initramfs-virt`, and `efi/boot/bootaa64.efi` from the ISO, appends the eight
  vsock/virtio-net/overlay modules to the initramfs (Alpine's own virt image has `virtiofs`/`fuse`
  but no vsock or network driver modules), and emits a deterministic 128 MiB FAT32 ESP — fixed
  volume id/label, fixed DOS epoch timestamps (fatfs without `chrono`), files in a fixed order, so
  two runs over unchanged inputs produce byte-identical bytes (`b38afd5c`). A sha256 manifest
  (`manifest.json`) beside the image gates the runtime preflight.
- The rootfs is provisioned by the same command: it cross-compiles `miniboxd` for
  `aarch64-unknown-linux-musl`, stages it as `<vm_dir>/rootfs/sbin/minibox-agent`, installs
  busybox `inittab`/`rcS` (the same mechanism the isolation suite relies on), and copies the
  eight guest modules into `<vm_dir>/rootfs/lib/modules/`.
- `minibox_core::vm_boot::kernel_cmdline()` renders `console=hvc0 root=minibox-rootfs
rootfstype=virtiofs rw quiet modules=vsock,vmw_vsock_virtio_transport_common,vmw_vsock_virtio_transport,failover,net_failover,virtio_net,af_packet,overlay`
  — the same value is baked into the ESP's `/boot/grub/grub.cfg`. Both the builder and the
  runtime read it, so they cannot drift.
- The ESP layout is the one planned: `/EFI/BOOT/BOOTAA64.EFI`, `/boot/grub/grub.cfg`,
  `/boot/{vmlinuz-virt,initramfs-virt}`. GRUB comes from the ISO's own `efi/boot/bootaa64.efi`
  rather than a separately pinned binary.
- Guest mount point is `/sysroot`, not `/newroot` as originally written.
- `modules=` on the cmdline asks the initramfs init to `modprobe` the listed modules; observed
  not to be reliable, so the rootfs's `rcS` also loads them explicitly with `insmod`, in
  dependency order, emitting per-module `OK:`/`FAIL:` reporting for the virtio-net failover chain
  plus `lsmod` and a `-- net diag --` block (`ip link`/`ip addr`/`ip route`, `/etc/resolv.conf`,
  virtio device enumeration, `dmesg` grep) — added in `538bfe90` to diagnose the virtio-net and
  overlayfs issues found during the signing/EFI work. That diagnostic output is not captured in
  a committed artifact, so treat any guest-side observation beyond "virtiofs root mounted" as
  unverified until `just test-vz-isolation` runs to completion.
- `rcS` brings `lo`/`eth0` up and runs `udhcpc -i eth0 -n -q` with 10 retries (eth0 may still
  be negotiating, or the host's VZNAT DHCP server may not be ready); the udhcpc default script
  (provisioned in the rootfs) writes `/etc/resolv.conf`. Whether the lease completes has not
  been re-verified. The provisioned rootfs has no `/sbin/openrc` by design — the agent boots
  via busybox init.
- The `vz` daemon must carry `com.apple.security.virtualization`; ad-hoc signing via
  `just sign-vz` suffices on macOS 26.x and `cargo xtask doctor` checks it.

Not yet verified: the isolation suite booting the guest (`just test-vz-isolation`), full
`mbx run`/`mbx console` client traffic over vsock, and the phase-2 smoke test. These are
tracked as follow-ups; see "Validation plan / acceptance criteria" below.

## New evidence: the entitlement confound

> Resolved by the Phase 0 run below: without the entitlement, `validateWithError` fails
> explicitly (`code=2`), and with it `VZLinuxBootLoader` still fails with `code=1`. The
> confound is closed; the text below is kept as the reasoning that motivated the experiment.

`VZErrorDomain code=1` is the framework's generic "something below me failed" code. It is
also exactly what a denied `mach-lookup com.apple.Virtualization.VirtualMachine` produces
when a process lacks the `com.apple.security.virtualization` entitlement (see Brent
Fitzgerald, "Starting a VM from a MacOS sandbox", 2026-05: `log stream` shows
`Sandbox: limactl(...) deny(1) mach-lookup com.apple.Virtualization.VirtualMachine` behind a
`Code=1`). Apple's own documentation states the entitlement is required to create a virtual
machine.

Three facts about this repo matter:

1. The only place an entitlement is ever applied is `Justfile`'s `test-vz-isolation` recipe,
   which codesigns the **test binary** with `entitlements/vz-test.entitlements`.
2. Nothing signs `miniboxd`. `cargo run`, `cargo build`, and `cargo nextest run` all produce
   binaries with no `com.apple.security.virtualization` entitlement, and CI
   (`.github/workflows/*.yml`) never codesigns anything.
3. The signed harness **never created a VM**: every isolation test reports `SKIP` because
   `~/.minibox/vm` does not exist, and `cargo xtask build-vm-image` — the command the skip
   message tells you to run — is not implemented anywhere in this codebase. The only working
   provisioner today is `cargo xtask test-linux`, which stages guest binaries into `rootfs/`
   and packs an initramfs from it, but expects `boot/vmlinuz-virt` to already exist.

So the "signed harness runs cleanly" data point is a data point about process startup, not
about Virtualization.framework. The one comparison that exists (Lima boots with
`VZEFIBootLoader`, minibox fails with `VZLinuxBootLoader`) varies **two** things at once:
boot loader and code signature. Lima's `limactl` ships signed with the virtualization
entitlement.

That is not enough to justify rewriting the boot path. It is enough to justify a two-hour
experiment.

## Phase 0: decision gate (required)

The 2026-08-15 repro existed only in session history, so it is committed as a feature-gated
example, `crates/macbox/examples/vz_boot_probe.rs` (built with `--features vz-probe`). It takes
`--loader linux|efi`, `--work-dir`, `--wait-secs`, `--hold-secs`, and `--disk PATH`, and prints
a single `PROBE_RESULT` line. A gate whose evidence artifact is not in the tree cannot be
re-run or reviewed later.

### Matrix

Signing means ad-hoc signing with `entitlements/vz-test.entitlements`; unsigned means a normal
`cargo`-built binary.

| Cell | Signing  | Boot loader                                          | Meaning if it boots                                           |
| ---- | -------- | ---------------------------------------------------- | ------------------------------------------------------------- |
| A    | unsigned | `VZLinuxBootLoader`                                  | Unexpected; record it and re-check the probe.                 |
| B    | entitled | `VZLinuxBootLoader`                                  | Root cause is signing. Cancel Phase 1.                        |
| C    | unsigned | `VZEFIBootLoader` + `VZGenericPlatformConfiguration` | Signing and boot loader both matter; run D before concluding. |
| D    | entitled | `VZEFIBootLoader` + `VZGenericPlatformConfiguration` | EFI path is viable; proceed to Phase 1.                       |

Decision rules:

- **B boots** -> the adapter never needed an EFI change. Delete Phase 1 from this design and
  instead ship: daemon signing (`just sign-vz`), an entitlement preflight check, and a
  code=1 error hint. Smallest possible change.
- **B fails, D boots** -> EFI is required. Implement Phase 1 as specified below.
- **B and D both fail** -> stop. Neither mechanism works on this OS build; do not rewrite the
  boot path against unproven machinery. Record the finding and leave `vz` blocked.
- **B boots, D fails** -> EFI is a regression here; keep the kernel path and do not adopt
  Phase 1.

### Result (executed 2026-10-22)

Host: Apple M5 Pro, macOS 26.4 (build 25E246), arm64. Probe binary built from this tree,
copied twice: one copy left as built, one re-signed ad-hoc with the virtualization entitlement
(verified with `codesign -d --entitlements :-`). Guest assets: Alpine v3.22 aarch64
`vmlinuz-virt`, plus a 64 MiB FAT32 ESP whose `EFI/BOOT/BOOTAA64.EFI` is that same kernel
(the arm64 kernel is an EFI stub), so cells B and D boot the identical kernel.

| Cell | Signing  | Loader | Disk         | `start` outcome                                                                                                                              | Observed state                                  |
| ---- | -------- | ------ | ------------ | -------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------- |
| A    | unsigned | Linux  | none         | `configuration validation failed`, `VZErrorDomain code=2`: "The process doesn't have the \"com.apple.security.virtualization\" entitlement." | never started                                   |
| B    | entitled | Linux  | ESP attached | `start=error domain=VZErrorDomain code=1 description=Internal Virtualization error. The virtual machine failed to start.` after 102 ms       | never ran                                       |
| C    | unsigned | EFI    | ESP          | same explicit entitlement failure as A, `code=2`                                                                                             | never started                                   |
| D    | entitled | EFI    | ESP          | `start=ok` after ~100 ms                                                                                                                     | `Running` for the full 20 s hold, no early exit |

Two controls were run to remove confounds:

- Cell D with a zeroed 8 MiB raw disk (no FAT, nothing bootable) started `ok` but reached
  `Stopped` (state 0) within 210 ms — firmware had nothing to launch and powered off. With the
  real ESP it stayed `Running`. That difference is firmware actually launching the EFI
  application.
- Cell B was re-run with the same ESP attached, so storage layout is identical between B and D.
  It still failed with `code=1` in 102 ms.

### Conclusion

The gate resolves to **B fails, D boots**: proceed with Phase 1. Three findings drive the rest
of this design:

1. The entitlement is mandatory and is checked up front: without it, `validateWithError` fails
   with `code=2` and a message naming the entitlement, never reaching `start`. So `code=1` from
   the shipped adapter was never an entitlement problem, and the 2026-08-15 conclusion that the
   fault lies above signing is confirmed rather than overturned. Note that `code=2` is
   `invalidVirtualMachineConfiguration` and covers any rejected configuration — the shipped
   adapter reports it for an invalid boot loader too — so the accompanying message, not the
   code, identifies the entitlement as the cause.
2. `VZLinuxBootLoader` is what fails, deterministically, on macOS 26.4 / arm64. The same kernel
   boots through `VZEFIBootLoader` with the same memory, CPU count, and storage.
3. Ad-hoc entitlements are honoured on macOS 26.4. Developer ID signing is not required to use
   Virtualization.framework locally, so `just sign-vz` can stay a local ad-hoc step.

Serial output was empty in every cell, including D. That is expected, not a failure: without a
boot manager there is no `console=` on the kernel command line, so the guest kernel has no
console to write to. Phase 1's boot manager supplies it, which is why the serial console
becomes a Phase 1 acceptance criterion rather than a Phase 0 one.

### Follow-ups this raises

- T1 (daemon entitlement + `just sign-vz` + `cargo xtask doctor` check) is still required: cell
  A/C show an unsigned daemon cannot even validate a configuration. **Done** — see the T1 row.
  Because `code=2` is generic (`invalidVirtualMachineConfiguration`, also raised for an invalid
  boot loader), remediation text must quote the entitlement message rather than keying off the
  code alone; T4's `code=1` trailer keeps that rule.
- The ad-hoc-vs-Developer-ID scope question is answered for local use (ad-hoc works). Release
  signing is unchanged and out of scope here.

## Goal (Phase 1, selected by Phase 0)

Boot the guest through `VZEFIBootLoader` from a FAT32 EFI system partition (ESP) image, while
keeping the existing guest contract unchanged: virtiofs `minibox-rootfs` / `minibox-images` /
`minibox-containers` shares, the vsock agent on port 9000, the serial console on `hvc0`, and
the busybox init that execs `minibox-agent`.

## Non-goals

- Making `vz` the default macOS adapter, or changing `smolvm`/`krun`/`colima` selection.
- A full disk-image rootfs. The rootfs stays a virtiofs share (see "Deferred alternative").
- Graphics (`VZVirtualMachineView`), Rosetta directory share, snapshots, suspend/resume, or
  nested virtualization. (Networking devices were originally listed here as a non-goal, but the
  as-built adapter attaches a virtio-net device with NAT — see the Boot mechanism section.)
- Guaranteeing `vz` works in CI. Booting a VM in CI requires the entitlement and nested
  virtualization; Phase 1 validates locally and records the OS build.

## Boot mechanism

`VZEFIBootLoader` takes exactly one input: a `VZEFIVariableStore`. It exposes no way to pass a
kernel command line or an initrd URL. Firmware therefore needs a boot manager on the ESP that
can load both. As built, GRUB (`BOOTAA64.EFI`) comes from the pinned Alpine ISO's own
`efi/boot/bootaa64.efi` rather than a separately pinned GRUB binary:

```text
ESP (FAT32, read-only attachment, 128 MiB)
  /EFI/BOOT/BOOTAA64.EFI   <- grubaa64.efi (from pinned Alpine ISO)
  /boot/grub/grub.cfg
  /boot/vmlinuz-virt
  /boot/initramfs-virt
```

`grub.cfg` (generated at build time; the `modules=` parameter was added during bring-up — see
the as-built notes above):

```text
set timeout=0
set default=0
menuentry "minibox" {
  linux /boot/vmlinuz-virt console=hvc0 root=minibox-rootfs rootfstype=virtiofs rw quiet \
    modules=vsock,vmw_vsock_virtio_transport_common,vmw_vsock_virtio_transport,failover,net_failover,virtio_net,af_packet,overlay
  initrd /boot/initramfs-virt
}
```

The kernel command line moves out of `VZLinuxBootLoader::setCommandLine` and into `grub.cfg`.
Both boot modes render it from one shared function so the two paths cannot drift (as built):

```rust
// crates/minibox-core/src/vm_boot.rs — minibox_core::vm_boot::kernel_cmdline()
"console=hvc0 root=minibox-rootfs rootfstype=virtiofs rw quiet modules=vsock,vmw_vsock_virtio_transport_common,vmw_vsock_virtio_transport,failover,net_failover,virtio_net,af_packet,overlay"
```

`grub.cfg` is generated, not checked in, so `minibox_core::vm_boot::kernel_cmdline()` is the
only source of truth. One part of the original plan changed: the non-goals list excluded
networking devices, but the as-built adapter attaches a virtio-net device (the guest's rcS
brings `eth0` up and runs `udhcpc`), because the provisioned rootfs expects network access for
the isolation tests.

### Why a boot manager rather than the kernel as an EFI application

On arm64 the kernel `Image` is itself a PE/COFF EFI application, so `/EFI/BOOT/BOOTAA64.EFI`
could be `vmlinuz-virt` directly. That only works when the command line and initrd are
embedded in the image or supplied as EFI load options — neither of which
`VZEFIBootLoader` can do. Upstream Alpine kernels do not embed minibox's command line. A boot
manager is the smallest correct mechanism.

### Deferred alternative: full rootfs disk

A second viable shape is Lima's: keep a writable rootfs inside a disk image with its own
ESP, and drop the virtiofs root. It removes the "EFI + virtiofs root" combination (the least
proven part of this design), but it loses the host-visible `rootfs/` directory that
`cargo xtask test-linux` stages guest binaries into and that `vm_image_available()` in the
isolation suite checks, and it turns the rootfs into another image artifact to build,
snapshot, and garbage-collect. Defer it. If Phase 1's EFI-plus-virtiofs combination fails to
boot but EFI-plus-disk-rootfs does, this is the fallback, not the starting point.

## VZ object graph

As built: the machine-identifier block was not needed and is not wired up — a plain
`VZGenericPlatformConfiguration` plus the variable store boots the Linux guest. The graph:

```rust
// crates/macbox/src/vz/vm.rs — EFI branch of prepare_on_main_queue
let platform = unsafe { VZGenericPlatformConfiguration::new() };
// VZEFIBootLoader requires a VZGenericPlatformConfiguration, and that platform
// configuration must not be combined with a macOS (VZMac*) platform configuration.
unsafe { vm_config.setPlatform(&platform) };

let vars_url = path_to_nsurl(&config.efi_vars_path())?;
let store = if config.efi_vars_path().exists() {
    unsafe { VZEFIVariableStore::initWithURL(VZEFIVariableStore::alloc(), &vars_url) }
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

let boot_loader = unsafe { VZEFIBootLoader::new() };
unsafe { boot_loader.setVariableStore(Some(&store)) };
unsafe { vm_config.setBootLoader(Some(&*boot_loader)) };

// ESP is immutable at runtime; NVRAM lives in efi/NVRAM/efi_vars.fd, not on the FAT volume.
let esp_url = path_to_nsurl(&config.efi_boot_image_path())?;
let attachment = unsafe {
    VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_error(
        VZDiskImageStorageDeviceAttachment::alloc(),
        &esp_url,
        true,
    )
}.map_err(|e| anyhow!("attach EFI disk image: {e:?}"))?;
let block = unsafe { VZVirtioBlockDeviceConfiguration::initWithAttachment(
    VZVirtioBlockDeviceConfiguration::alloc(),
    &attachment,
) };
unsafe {
    vm_config.setStorageDevices(&NSArray::from_retained_slice(&[block.into_super()]));
}

// Optional but cheap: guest entropy on EFI boot has no virtio-rng today.
let rng = unsafe { VZVirtioEntropyDeviceConfiguration::new() };
unsafe { vm_config.setEntropyDevices(&NSArray::from_retained_slice(&[rng.into_super()])); }
```

Cargo feature additions to `crates/macbox/Cargo.toml` (`objc2-virtualization` is already
feature-gated per class; none of these are enabled today). As built, the active set is
`VZEFIBootLoader`, `VZEFIVariableStore`, `VZPlatformConfiguration`,
`VZGenericPlatformConfiguration`, `VZStorageDeviceConfiguration`,
`VZDiskImageStorageDeviceAttachment`, `VZVirtioBlockDeviceConfiguration`, and
`VZVirtioNetworkDeviceConfiguration`/`VZNATNetworkDeviceAttachment`/
`VZVirtioSocketDeviceConfiguration`; the `VZGenericMachineIdentifier` block was not needed
and is not wired up, and entropy devices are not attached.

Threading is unchanged: all of the above still happens inside
`VzVm::prepare_on_main_queue`, called through `dispatch_sync_f` from `vz_main()`.

## Boot asset layout

As built (`~/.minibox/vm/`):

```text
~/.minibox/vm/
  boot/
    vmlinuz-virt              (extracted from the pinned ISO)
    initramfs-virt            (extracted, then augmented with the 8 guest modules)
  efi/
    boot.img                  (FAT32 ESP, built by xtask)
    manifest.json             (per-file + ESP sha256, cmdline, pinned input hashes)
    NVRAM/efi_vars.fd         (EFI NVRAM variable store, created at first boot)
  rootfs/                     (virtiofs; provisioned by build-vm-image — agent, rcS, modules)
  cache/                      (downloaded pinned artifacts: the ISO and the linux-virt APK)
```

Deltas from the original plan: no separate `grub/grubaa64.efi` (GRUB is read from the pinned
ISO), the sha256 manifest is `manifest.json` (not `boot.img.sha256`), the variable store is
`efi/NVRAM/efi_vars.fd`, and there is no `machine-id` — `VZGenericMachineIdentifier` was not
needed to boot the Linux guest and is not wired up.

- `boot.img` is a 128 MiB fixed-size image (Alpine kernel ~10 MB, initramfs ~10 MB,
  GRUB ~1 MB). FAT32 with fixed volume id/label and DOS epoch timestamps for determinism.
- `NVRAM/efi_vars.fd` is per-VM state under `vm_dir`. It contains no secrets, but it is
  boot-influencing state: delete it to return to a factory boot.
- The ESP is attached read-only. Nothing at runtime writes to it, so a mutable boot volume
  cannot be used to persist inside the guest.

## Provisioning: `cargo xtask build-vm-image`

Restore the command name the isolation suite already tells you to run. As built, the command
owns the **entire** VM asset tree, not just kernel/initramfs/ESP: it extracts the pinned ISO
(kernel/initramfs/GRUB), appends the eight vsock/virtio-net/overlay modules to the initramfs
(Alpine virt does not ship them), builds the deterministic ESP plus `manifest.json`, and
provisions the virtiofs rootfs itself (cross-compiling `miniboxd` for
`aarch64-unknown-linux-musl` and staging it as `rootfs/sbin/minibox-agent`, plus busybox init
files and the eight guest modules under `rootfs/lib/modules/`). The plan had
`build-vm-image` not touching `rootfs/`; that premise did not survive contact with the
module-load gap — see the as-built section. `cargo xtask test-linux` still owns its own
initramfs pipeline for the QEMU harness; the two are separate paths with different guest
contents.

Inputs are pinned by release and sha256 (the Alpine 3.22.7 virt ISO and the `linux-virt`
6.12.114-r0 APK for the module tree) — nothing is fetched by "latest". It renders `grub.cfg`
from `minibox_core::vm_boot::kernel_cmdline()`, formats a FAT32 volume, writes the four files,
and emits `boot.img` plus `manifest.json`.

Implementation notes (as built):

- Builds the FAT image in-process with the `fatfs` crate (no root, no `hdiutil`, no loopback).
  `fatfs` is built with `default-features = false` (`alloc` + `std` only) — its default
  `chrono` feature stamped directory entries with `Local::now()`, which broke byte determinism
  until `b38afd5c`. The `mtools` fallback was not needed.
- Deterministic output: fixed volume id (`0x4d42_5845`), fixed label (`MINIBOX`), fixed DOS
  epoch timestamps, files written in fixed order, so re-running on unchanged inputs yields
  identical bytes and an unchanged sha256. Unit test:
  `xtask::vm_image::tests::fat_image_is_deterministic_and_contains_every_file`.
- Idempotent: if the pinned inputs manifest matches, it prints a cached line and skips the
  rebuild; `--force` rebuilds.
- Missing or hash-mismatched pinned artifacts are hard errors with the expected URL and
  sha256, not silent skips.
- The runtime preflight (`macbox::vz::boot::EfiBootAssets::check_boot_assets`) reads
  `manifest.json` and refuses to start the VM on missing files or an `esp_sha256` mismatch,
  printing ``run `cargo xtask build-vm-image` `` as the remedy.

`cargo xtask doctor` has a `vz` section reporting kernel/initramfs/ESP presence and whether
the **current `miniboxd` binary** carries `com.apple.security.virtualization` (via
`codesign -d --entitlements :- <exe>`).

## Runtime changes

As built (`VzBootMode` is now a single-variant enum — the `LinuxKernel` mode it once carried
was deleted when the boot-loader fix landed):

```rust
pub enum VzBootMode {
    /// VZEFIBootLoader — GRUB on a FAT32 ESP, command line via grub.cfg.
    Efi,
}

pub struct EfiBootAssets {
    pub esp_image: PathBuf,     // vm_dir/efi/boot.img
    pub variable_store: PathBuf, // vm_dir/efi/NVRAM/efi_vars.fd
}
```

Boot mode resolution, evaluated once in `start_vz()`:

1. `MINIBOX_VZ_BOOT=efi` is required to be set to "efi" (or unset); any other value is a hard
   error (`validate_boot_mode_env` in `crates/macbox/src/vz/boot.rs`).
2. The boot assets are checked via `EfiBootAssets::check_boot_assets()` before `start`; a
   missing/mismatched `boot.img` fails closed with the `cargo xtask build-vm-image` remedy.

Storage and device wiring as built (vm.rs): read-only `VZDiskImageStorageDeviceAttachment`
around `boot.img` + `VZVirtioBlockDeviceConfiguration` for the ESP, a
`VZVirtioNetworkDeviceConfiguration` with `VZNATNetworkDeviceAttachment` (so the guest can
resolve DNS and pull images), `VZVirtioSocketDeviceConfiguration` via `setSocketDevices`
(the agent is reached over vsock on port 9000), and `VZSerialPortConfiguration` for the hvc0
serial output that the boot triage table reads. `prepare_on_main_queue` builds the boot loader
and sets the platform + variable store + storage devices; `boot_mode` is validated once in
`start_vz()` by `validate_boot_mode_env`.

`prepare_on_main_queue` builds the boot loader and sets the platform + variable store + storage
devices; the full `VzVmConfig`-equivalent is retained on `VzVm` for logging and diagnostics.

## Preflight and diagnostics

Today a missing image produces a silent `SKIP` in tests and a `code=1` in the daemon. Replace
that with a fail-loud check before `validateWithError`:

```rust
/// Verify every boot asset for the selected mode exists and matches its manifest.
/// Returns one error naming each missing/mismatched path and the exact remedy.
fn check_boot_assets(config: &VzVmConfig) -> Result<()>;
```

Failure text must name the file and the command: `run cargo xtask build-vm-image`.

Every `VZErrorDomain code=1` surfaced by `wait_for_running` gains a remediation trailer listing
the three known causes in order of likelihood: missing entitlement (with the `codesign`
command that fixes it), missing boot assets, and an OS-level VZ regression (with the
`log stream` predicate that distinguishes them).

Boot triage table, for `GOTCHAS.mbx.md` and the adapter docs:

| Symptom (serial console)                     | Cause                                                  |
| -------------------------------------------- | ------------------------------------------------------ |
| No output at all, `code=1` at start          | Entitlement / start refused before firmware ran.       |
| Firmware banner, then "no bootable device"   | ESP layout wrong; `BOOTAA64.EFI` not at the path.      |
| GRUB prompt or `error: file not found`       | `grub.cfg` paths or the kernel/initrd are missing.     |
| Kernel panic, `VFS: Unable to mount root fs` | `minibox-rootfs` virtiofs tag absent or cmdline drift. |
| Boots, agent unreachable on vsock            | Guest init changed; not a boot-loader problem.         |

## Signing and packaging

Regardless of which boot loader wins Phase 0, `vz` only works if the binary that calls
Virtualization.framework is signed with `com.apple.security.virtualization`.

- Add `entitlements/vz-daemon.entitlements` (same single key; separate file so the test
  harness keeps its own lifecycle) and a `just sign-vz` recipe that ad-hoc signs
  `target/{debug,release}/miniboxd`.
- In-process detection: `cargo xtask doctor` check (above) plus a one-line startup log in
  `start_vz()` stating whether the running process is entitled, so a bare `cargo run` failure
  is self-explanatory.
- Release builds need Developer ID signing with hardened runtime and the entitlement, then
  notarization. That packaging work is out of scope here, but the design must not assume
  ad-hoc signing is sufficient for distributed binaries — Phase 0 answers which is required.
- Tests keep using `entitlements/vz-test.entitlements`; do not delete the existing recipe.

## Security

- The ESP is read-only and hash-pinned; the guest cannot rewrite its own boot loader.
- `grub.cfg` uses `set timeout=0`. No GRUB shell entry, no editing, no recovery menu: the
  serial console is a host-owned channel, and an interactive boot prompt is an unnecessary
  second entry point.
- GRUB is pinned as part of the pinned Alpine ISO (whose sha256 is recorded in
  `manifest.json`), and the runtime preflight (`check_boot_assets`) refuses to start the VM on
  a missing or hash-mismatched `esp_sha256`. Every per-file SHA-256 for the ESP is recorded in
  the manifest, so tampering with any of them fails closed.
- `NVRAM/efi_vars.fd` is VM-local state under `vm_dir`; it contains no secrets, but it is
  boot-influencing state (EFI boot order). Delete it to return to a factory boot.
- The virtiofs shares keep their current read-only/read-write split; this design adds no new
  host-path exposure.

## Risk

| Risk                                                               | Likelihood | Mitigation                                                                                                |
| ------------------------------------------------------------------ | ---------- | --------------------------------------------------------------------------------------------------------- |
| EFI boot still returns `code=1` (OS-level VZ regression is real)   | Medium     | **Resolved by Phase 0**: cell D starts and runs.                                                          |
| EFI + virtiofs root does not work (proven part is EFI + disk root) | Medium     | Fallback is the full-disk rootfs variant; decide after one boot attempt.                                  |
| GRUB artifact supply chain / licensing                             | Low        | Pin URL + sha256; download at provisioning time rather than vendoring the GPL-3 binary into the repo.     |
| Command line drifts between the ESP `grub.cfg` and the runtime     | Low        | Single `kernel_cmdline()` source in `minibox_core::vm_boot` rendered by both the builder and the adapter. |
| FAT32 image built non-deterministically, breaking the hash gate    | Medium     | Fixed timestamps/label/ordering; unit test asserts two builds are byte-identical.                         |
| `fatfs` cannot format FAT32 at the chosen size                     | Low        | `mtools` fallback, documented as a dev dependency.                                                        |
| Signing requirement makes local dev awkward                        | High       | `just sign-vz`, doctor check, startup log, and an error hint that names the command.                      |
| EFI boot adds latency                                              | Low        | Budget +2s; measure in the smoke test and record it.                                                      |
| Docs drift (`FEATURE_MATRIX`, `ARCHITECTURE`, `context.toml`)      | Medium     | Task T6 updated maturity, adapter notes, and `[[profiles]]`; doc-audit gates run on commit.               |

## Validation plan / acceptance criteria

1. Phase 0 matrix recorded in this document with per-cell outcomes and the exact macOS build.
   **Met** (see "Result (executed 2026-10-22)").
2. `cargo xtask build-vm-image` is idempotent and produces identical bytes across two runs on
   unchanged inputs; the `manifest.json` hashes match on re-run. **Met** (as of `b38afd5c`,
   via `vm_image::tests::fat_image_is_deterministic_and_contains_every_file`, 8/8 passing).
3. `just test-vz-isolation` boots a real VM (no `SKIP`), with every isolation test passing or
   failing for a documented reason. **Not met** — not run since the EFI switch.
4. The smoke path reaches the guest agent over vsock within the existing 60s budget, with
   serial output showing firmware → GRUB → kernel → init. **Not met / unverified** — see
   "As-built status" (guest runtime behavior past virtiofs root mount is not captured).
5. `MINIBOX_ADAPTER=vz mbx run alpine echo ok` succeeds end to end on a signed daemon. **Not met / unverified.**
6. `FEATURE_MATRIX.mbx.md` records `vz` as `experimental` (os build + evidence) and
   `cargo xtask verify` passes. **Met** via this change; verify gates run on commit.

## Implementation outline (ordered, one commit each)

| Task | Scope                                                                                                         | Files                                                                                                                                                  |
| ---- | ------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| T0   | Phase 0 matrix experiment + record results; update this doc's status. **Done**                                | `crates/macbox/examples/vz_boot_probe.rs` (new)                                                                                                        |
| T1   | Signing plumbing: daemon entitlement file, `just sign-vz`, doctor check, startup entitlement log. **Done**    | `entitlements/vz-daemon.entitlements` (new), `Justfile`, `xtask/src/preflight.rs`, `crates/macbox/src/vz/signing.rs` (new), `crates/macbox/src/lib.rs` |
| T2   | `cargo xtask build-vm-image`: ESP builder, grub rendering, sha256 manifest. **Done** (`4cd2fa47`)             | `xtask/src/vm_image.rs`, `xtask/src/main.rs`                                                                                                           |
| T3   | EFI boot loader branch, platform config, storage devices, cargo features. **Done** (`4cd2fa47`)               | `crates/macbox/Cargo.toml`, `crates/macbox/src/vz/vm.rs`, `crates/macbox/src/vz/boot.rs`                                                               |
| T4   | Asset preflight, boot-mode validation, code=1 remediation text. **Done** (`6d8c009a`)                         | `crates/macbox/src/vz/vm.rs`, `crates/macbox/src/lib.rs`, `crates/macbox/src/vz/boot.rs`                                                               |
| T5   | Tests: ESP builder units, boot-mode unit tests pass; isolation suite + smoke unverified                       | `xtask/src/vm_image.rs`, `crates/macbox/src/vz/`, `crates/macbox/tests/`                                                                               |
| T6   | Docs and maturity: feature matrix, gotchas triage table, `xtask/context.toml` profile. **Done** (this change) | `docs/core/`, `xtask/context.toml`, `DEVELOPMENT.md`                                                                                                   |

T1 is required regardless: cells A and C show an unsigned process cannot even validate a
configuration, so the daemon must be signed before any `vz` boot can be attempted.

## Open questions

1. Does the ESP need a written EFI boot entry, or does firmware fall back to
   `/EFI/BOOT/BOOTAA64.EFI` on an empty NVRAM? (Expected: fallback works; verify with a fresh
   `NVRAM/efi_vars.fd`.) **Answered**: a freshly created variable store with no boot entries
   booted `/EFI/BOOT/BOOTAA64.EFI` unaided; the VM stayed running with a real ESP and powered off
   within 210 ms with an unbootable one.
2. Is `modules=` on the kernel cmdline a viable mechanism, or must the rootfs `rcS` insmod the
   modules explicitly? **Answered (as built)**: `modules=` was observed not to be reliable
   (`socket(AF_VSOCK)` and `AF_PACKET` still failed after `switch_root` during the
   signing/EFI bring-up), so `rcS` now `insmod`s the eight modules explicitly in dependency
   order, with per-module `OK:`/`FAIL:` diagnostics (`lsmod` + `-- net diag --`), and the ESP
   builder appends the same modules to the initramfs as a second line of defense. Mitigation,
   not yet verified to completion — see "As-built status" above.
3. Is a read-only `VZDiskImageStorageDeviceAttachment` accepted for the boot volume? **Answered**:
   yes; cells D and B used `readOnly = true`.
4. Ad-hoc versus Developer ID entitlements on macOS 26. **Answered**: ad-hoc entitlements are
   honoured; the entitled ad-hoc binary reached a running VM.
5. GRUB versus systemd-boot as the pinned boot manager: **Answered as built** — GRUB is
   sourced from the pinned Alpine ISO's `efi/boot/bootaa64.efi` rather than a separately pinned
   binary (one artifact fewer to pin). Why GRUB and not systemd-boot: its `linux`/`initrd`
   commands map exactly onto the existing kernel+initramfs artifacts, while systemd-boot needs
   the kernel to be an EFI-stub binary and a different config format. Note that Phase 0 booted
   the kernel directly as `BOOTAA64.EFI`, which proves the EFI stub works but supplies no
   `console=` or `root=`, so it cannot replace a boot manager. The systemd-boot alternative
   remains possible if GRUB's binary ever proves hard to source.
