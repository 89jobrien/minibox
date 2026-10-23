# Design: VZ EFI Boot Path

**Status**: proposal, not approved. The Phase 0 gate has been **executed and passed** (cell D;
see "Result" below), which selects Phase 1 as the implementation path. Phase 1 itself is not
approved or started.

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
ruled out. The conclusion still rests on one uncontrolled variable, described below, which is
cheaper to test than the boot-mechanism rewrite it currently implies.

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
`minibox-containers` shares, the vsock agent on port 1024, the serial console on `hvc0`, and
the busybox init that execs `minibox-agent`.

## Non-goals

- Making `vz` the default macOS adapter, or changing `smolvm`/`krun`/`colima` selection.
- A full disk-image rootfs. The rootfs stays a virtiofs share (see "Deferred alternative").
- Graphics (`VZVirtualMachineView`), networking devices, Rosetta directory share, snapshots,
  suspend/resume, or nested virtualization.
- Guaranteeing `vz` works in CI. Booting a VM in CI requires the entitlement and nested
  virtualization; Phase 1 validates locally and records the OS build.

## Boot mechanism

`VZEFIBootLoader` takes exactly one input: a `VZEFIVariableStore`. It exposes no way to pass a
kernel command line or an initrd URL. Firmware therefore needs a boot manager on the ESP that
can load both. Use GRUB's AArch64 EFI build:

```text
ESP (FAT32, read-only attachment)
  /EFI/BOOT/BOOTAA64.EFI   <- grubaa64.efi (pinned upstream artifact)
  /boot/grub/grub.cfg
  /boot/vmlinuz-virt
  /boot/initramfs-virt
```

`grub.cfg`:

```text
set timeout=0
set default=0
menuentry "minibox" {
  linux /boot/vmlinuz-virt console=hvc0 root=minibox-rootfs rootfstype=virtiofs rw quiet
  initrd /boot/initramfs-virt
}
```

The kernel command line moves out of `VZLinuxBootLoader::setCommandLine` and into `grub.cfg`.
Both boot modes render it from one shared constant so the two paths cannot drift:

```rust
// crates/macbox/src/vz/boot.rs
pub const KERNEL_CMDLINE: &str =
    "console=hvc0 root=minibox-rootfs rootfstype=virtiofs rw quiet";
```

`grub.cfg` is generated, not checked in, so the constant is the only source of truth.

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

```rust
// crates/macbox/src/vz/vm.rs — EFI branch of prepare_on_main_queue
let platform = unsafe { VZGenericPlatformConfiguration::new() };
if let Some(id) = load_machine_identifier(&config)? {
    unsafe { platform.setMachineIdentifier(&id) };
}
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

// ESP is immutable at runtime; NVRAM lives in vars.fd, not on the FAT volume.
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
feature-gated per class; none of these are enabled today):

```text
VZEFIBootLoader, VZEFIVariableStore, VZPlatformConfiguration,
VZGenericPlatformConfiguration, VZGenericMachineIdentifier,
VZStorageDeviceConfiguration, VZStorageDeviceAttachment,
VZDiskImageStorageDeviceAttachment, VZVirtioBlockDeviceConfiguration,
VZEntropyDeviceConfiguration, VZVirtioEntropyDeviceConfiguration
```

Threading is unchanged: all of the above still happens inside
`VzVm::prepare_on_main_queue`, called through `dispatch_sync_f` from `vz_main()`.

## Boot asset layout

```text
~/.minibox/vm/
  boot/
    vmlinuz-virt              (existing)
    initramfs-virt            (existing)
    grub/grubaa64.efi         (pinned artifact, fetched once)
  efi/
    boot.img                  (FAT32 ESP, built by xtask)
    boot.img.sha256           (manifest; boot refuses on mismatch)
    vars.fd                   (EFI NVRAM, created at first boot)
    machine-id                (VZGenericMachineIdentifier dataRepresentation)
  rootfs/                     (virtiofs, unchanged)
```

- `boot.img` is a 128 MiB fixed-size image (Alpine kernel ~10-15 MB, initramfs ~5-10 MB,
  GRUB ~3 MB). FAT32 with a 512-byte sector size.
- `vars.fd` and `machine-id` are per-VM state under `vm_dir`, mode `0600`. They contain no
  secrets, but they are boot-influencing state: delete both to return to a factory boot.
- The ESP is attached read-only. Nothing at runtime writes to it, so a mutable boot volume
  cannot be used to persist inside the guest.

## Provisioning: `cargo xtask build-vm-image`

Restore the command name the isolation suite already tells you to run. It is the boot-asset
provisioner, not a second test-image pipeline: `crates/macbox/tests/vz_isolation_tests.rs` and
`scripts/qemu-linux-tests/run.rs` both consume
`~/.minibox/vm/boot/{vmlinuz-virt,initramfs-virt}`, while `cargo xtask test-linux` already
stages guest binaries into `rootfs/` and packs an initramfs from it
(`xtask/src/test_linux.rs`). `build-vm-image` therefore owns kernel/initramfs acquisition and
ESP assembly, and must not duplicate the rootfs staging that already exists.

Inputs: existing Alpine `vmlinuz-virt` / `initramfs-virt` (or the pinned upstream URLs when
absent), the pinned `grubaa64.efi`, and the ESP size. Alpine artifacts are pinned by release
and sha256 (the 2026-08-15 repro used the official v3.22 aarch64 `netboot` pair), not fetched
by "latest". It renders `grub.cfg` from `vz::boot::KERNEL_CMDLINE`, formats a FAT32 volume,
writes the four files, and emits `boot.img` plus `boot.img.sha256`. It never touches `rootfs/`.

Implementation notes:

- Build the FAT image in-process with the `fatfs` crate (`format_volume` + `create_dir` +
  `File::create`). No root, no `hdiutil`, no loopback mount, works identically on macOS and
  Linux CI. If `fatfs` cannot produce a FAT32 volume at this size, fall back to `mtools`
  (`mformat`/`mcopy`) and make it a documented dev dependency.
- Deterministic output: fixed volume label, fixed timestamps, files written in sorted order,
  so re-running on unchanged inputs yields identical bytes and an unchanged sha256. That makes
  the hash meaningful for the runtime preflight.
- Idempotent: if `boot.img.sha256` matches the inputs' manifest, skip the rebuild.
- Missing `grubaa64.efi` is a hard error with the expected URL and sha256, not a silent skip.

`cargo xtask doctor` gains a `vz` section that reports: kernel/initramfs present, ESP present
and hash-matching, `vars.fd` state, and whether the **current `miniboxd` binary** carries
`com.apple.security.virtualization` (shell out to
`codesign -d --entitlements :- <exe>` and look for the key).

## Runtime changes

`crates/macbox/src/vz/vm.rs`:

```rust
pub enum VzBootMode {
    /// VZLinuxBootLoader — kernel + initrd paths, command line via setCommandLine.
    LinuxKernel,
    /// VZEFIBootLoader — GRUB on a FAT32 ESP, command line via grub.cfg.
    Efi,
}

pub struct VzVmConfig {
    pub vm_dir: PathBuf,
    pub images_dir: PathBuf,
    pub containers_dir: PathBuf,
    pub memory_bytes: u64,
    pub cpu_count: usize,
    pub boot_mode: VzBootMode,
}

impl VzVmConfig {
    pub fn efi_boot_image_path(&self) -> PathBuf;   // vm_dir/efi/boot.img
    pub fn efi_vars_path(&self) -> PathBuf;         // vm_dir/efi/vars.fd
    pub fn efi_machine_id_path(&self) -> PathBuf;   // vm_dir/efi/machine-id
}
```

Boot mode resolution order, evaluated once in `start_vz()`:

1. `MINIBOX_VZ_BOOT=efi|kernel` if set (invalid value is a hard error, not a fallback).
2. `Efi` when `vm_dir/efi/boot.img` exists.
3. `LinuxKernel` otherwise (preserves current behaviour on a provisioned kernel-only image).

`prepare_on_main_queue` splits into `build_linux_boot_loader` and `build_efi_boot_loader`, both
returning `Retained<VZBootLoader>`, then calls the shared `setPlatform` path. `boot_mode` and
the full `VzVmConfig` are retained on `VzVm` for logging and diagnostics (the existing
`#[allow(dead_code)] config` field stops being dead).

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
- GRUB is a pinned artifact with a recorded sha256, fetched by an explicit provisioning step,
  never at boot. A missing or mismatched binary fails closed.
- `vars.fd` and `machine-id` are VM-local state, `0600`, under `vm_dir` alongside the rootfs.
- The virtiofs shares keep their current read-only/read-write split; this design adds no new
  host-path exposure.

## Risk

| Risk                                                               | Likelihood | Mitigation                                                                                            |
| ------------------------------------------------------------------ | ---------- | ----------------------------------------------------------------------------------------------------- |
| EFI boot still returns `code=1` (OS-level VZ regression is real)   | Medium     | **Resolved by Phase 0**: cell D starts and runs.                                                      |
| EFI + virtiofs root does not work (proven part is EFI + disk root) | Medium     | Fallback is the full-disk rootfs variant; decide after one boot attempt.                              |
| GRUB artifact supply chain / licensing                             | Low        | Pin URL + sha256; download at provisioning time rather than vendoring the GPL-3 binary into the repo. |
| Command line drifts between boot modes                             | Low        | Single `KERNEL_CMDLINE` constant rendered into `grub.cfg`.                                            |
| FAT32 image built non-deterministically, breaking the hash gate    | Medium     | Fixed timestamps/label/ordering; unit test asserts two builds are byte-identical.                     |
| `fatfs` cannot format FAT32 at the chosen size                     | Low        | `mtools` fallback, documented as a dev dependency.                                                    |
| Signing requirement makes local dev awkward                        | High       | `just sign-vz`, doctor check, startup log, and an error hint that names the command.                  |
| EFI boot adds latency                                              | Low        | Budget +2s; measure in the smoke test and record it.                                                  |
| Docs drift (`FEATURE_MATRIX`, `ARCHITECTURE`, `context.toml`)      | Medium     | Task T6 updates maturity, adapter notes, and `[[profiles]]`.                                          |

## Validation plan / acceptance criteria

1. Phase 0 matrix recorded in this document with per-cell outcomes and the exact macOS build.
   **Met** (see "Result (executed 2026-10-22)").
2. If Phase 1 proceeds: `cargo xtask build-vm-image` is idempotent and produces identical
   bytes across two runs; `boot.img.sha256` matches on re-run.
3. `just test-vz-isolation` boots a real VM (no `SKIP`), with every isolation test passing or
   failing for a documented reason.
4. `vz_adapter_smoke.rs` reaches the guest agent over vsock within the existing 60s budget,
   with serial output showing firmware -> GRUB -> kernel -> init.
5. `MINIBOX_ADAPTER=vz mbx run alpine echo ok` succeeds end to end on a signed daemon.
6. `cargo xtask verify` passes; `FEATURE_MATRIX.mbx.md` moves `vz` from `blocked` to
   `experimental` with the OS build and evidence link, or stays `blocked` with the Phase 0
   finding recorded.

## Implementation outline (ordered, one commit each)

| Task | Scope                                                                                                      | Files                                                                                                                                                  |
| ---- | ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| T0   | Phase 0 matrix experiment + record results; update this doc's status. **Done**                             | `crates/macbox/examples/vz_boot_probe.rs` (new)                                                                                                        |
| T1   | Signing plumbing: daemon entitlement file, `just sign-vz`, doctor check, startup entitlement log. **Done** | `entitlements/vz-daemon.entitlements` (new), `Justfile`, `xtask/src/preflight.rs`, `crates/macbox/src/vz/signing.rs` (new), `crates/macbox/src/lib.rs` |
| T2   | `cargo xtask build-vm-image`: ESP builder, grub rendering, sha256 manifest                                 | `xtask/src/vm_image.rs` (new), `xtask/src/main.rs`                                                                                                     |
| T3   | EFI boot loader branch, platform config, storage + entropy devices, cargo features                         | `crates/macbox/Cargo.toml`, `crates/macbox/src/vz/vm.rs`, new `vz/boot.rs`                                                                             |
| T4   | Asset preflight, boot-mode resolution, code=1 remediation text                                             | `crates/macbox/src/vz/vm.rs`, `crates/macbox/src/lib.rs`                                                                                               |
| T5   | Tests: ESP builder units, boot-mode unit tests, real-image isolation + smoke                               | `xtask/src/vm_image.rs`, `crates/macbox/src/vz/`, `crates/macbox/tests/`                                                                               |
| T6   | Docs and maturity: feature matrix, gotchas triage table, `xtask/context.toml` profile                      | `docs/core/`, `xtask/context.toml`, `DEVELOPMENT.md`                                                                                                   |

T1 is required regardless: cells A and C show an unsigned process cannot even validate a
configuration, so the daemon must be signed before any `vz` boot can be attempted.

## Open questions

1. Does the ESP need a written EFI boot entry, or does firmware fall back to
   `/EFI/BOOT/BOOTAA64.EFI` on an empty NVRAM? (Expected: fallback works; verify with a fresh
   `vars.fd`.) **Answered**: a freshly created `vars.fd` with no boot entries booted
   `/EFI/BOOT/BOOTAA64.EFI` unaided; the VM stayed running with a real ESP and powered off
   within 210 ms with an unbootable one.
2. Does Alpine's `initramfs-virt` load `virtiofs` before switch-root, given the root is
   virtiofs rather than a block device? The running `VZLinuxBootLoader` path proves the
   modules exist in the initramfs; it does not prove EFI ordering is identical.
3. Is a read-only `VZDiskImageStorageDeviceAttachment` accepted for the boot volume? **Answered**:
   yes; cells D and B used `readOnly = true`.
4. Ad-hoc versus Developer ID entitlements on macOS 26. **Answered**: ad-hoc entitlements are
   honoured; the entitled ad-hoc binary reached a running VM.
5. GRUB versus systemd-boot as the pinned boot manager: GRUB is larger but its `linux`/`initrd`
   commands map exactly onto the existing kernel+initramfs artifacts; systemd-boot needs the
   kernel to be an EFI-stub binary and a different config format. Default to GRUB unless the
   artifact proves hard to source. Note that Phase 0 booted the kernel directly as
   `BOOTAA64.EFI`, which proves the EFI stub works but supplies no `console=` or `root=`, so it
   cannot replace a boot manager.
