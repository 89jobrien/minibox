# QEMU Cross-Platform VM Runner Design

**Date:** 2026-04-23
**Branch:** `feat/qemu-cross-platform`
**Status:** Approved for implementation

## Problem

`xtask/src/vm_run.rs` hardcodes macOS-specific QEMU flags:

- Binary: `qemu-system-aarch64`
- Accelerator: `-accel hvf` (Apple Hypervisor.framework — macOS only)
- Alpine arch: `aarch64`
- Musl cross-compile target: `aarch64-unknown-linux-musl`

This blocks `cargo xtask test-vm` on Linux CI hosts where KVM is available and the
host may be x86_64. The fix is platform detection at runtime, not compile time.

## Goals

- `cargo xtask test-vm` works on Linux x86_64 and Linux arm64 CI hosts
- Same command works on macOS arm64 (no regression)
- Code is structured to support a future `QemuRuntime` adapter (Phase B)

## Non-Goals

- QEMU microvm machine type (Phase B concern)
- `MINIBOX_ADAPTER=qemu` runtime wiring (Phase B)
- TCG fallback (no KVM) — fail fast with a clear error; TCG is too slow for CI

## Platform Matrix

| Host                | QEMU binary            | Accelerator | Alpine arch | Musl target                    |
| ------------------- | ---------------------- | ----------- | ----------- | ------------------------------ |
| macOS arm64         | `qemu-system-aarch64`  | `hvf`       | `aarch64`   | `aarch64-unknown-linux-musl`   |
| Linux x86\_64       | `qemu-system-x86_64`   | `kvm`       | `x86_64`    | `x86_64-unknown-linux-musl`    |
| Linux arm64         | `qemu-system-aarch64`  | `kvm`       | `aarch64`   | `aarch64-unknown-linux-musl`   |

## Phase A: Accelerator + Arch Detection

### `HostPlatform` enum (`xtask/src/vm_run.rs`)

```rust
pub enum HostPlatform {
    MacOsArm64,
    LinuxX86_64,
    LinuxArm64,
}

impl HostPlatform {
    pub fn detect() -> Result<Self>   // uses std::env::consts::{OS, ARCH}
    pub fn qemu_binary(&self) -> &str
    pub fn accel(&self) -> &str
    pub fn alpine_arch(&self) -> &str
    pub fn musl_target(&self) -> &str
    pub fn machine_type(&self) -> &str   // "virt" for all — preserves current behaviour
}
```

Detection is pure `std::env::consts` — no subprocess, no new deps.

### Changes to `vm_run.rs`

- Remove `const QEMU_BASE_ARGS` — replaced by `HostPlatform` methods
- `run_vm_interactive(vm_dir)` → `run_vm_interactive(vm_dir, platform)`
- `test_vm(vm_dir, cargo_target)` → `test_vm(vm_dir, cargo_target, platform)`
- Callers in `xtask/src/main.rs` call `HostPlatform::detect()?` once and pass it down

### Changes to `vm_image.rs`

- `ALPINE_ARCH: &str = "aarch64"` constant → `HostPlatform::alpine_arch()`
- `build_agent()` cross-compile target → `HostPlatform::musl_target()`
- `build_vm_image` entry point detects platform and threads it through

### Error message for unsupported host

```
unsupported host: os=windows arch=x86_64
  QEMU VM runner requires macOS (hvf) or Linux (kvm).
```

## Phase C: `VmRunner` Struct (follow-on)

After Phase A is green on CI, extract the QEMU lifecycle into a `VmRunner` struct.
This lives in `xtask/src/vm_run.rs` (not a separate crate — xtask is a dev tool).

```rust
pub struct VmRunner {
    platform: HostPlatform,
    vm_dir: PathBuf,
    cargo_target: PathBuf,
}

impl VmRunner {
    pub fn new(platform: HostPlatform, vm_dir: PathBuf, cargo_target: PathBuf) -> Self
    pub fn run_tests(&self, suites: &[&str]) -> Result<()>
    pub fn run_interactive(&self) -> Result<()>
    /// Spawn VM, return handle. Entry point for Phase B adapter.
    pub fn spawn_vm(&self, kernel_cmdline: &str) -> Result<VmHandle>
}

pub struct VmHandle {
    child: std::process::Child,
    serial_sock: PathBuf,
}

impl VmHandle {
    pub fn connect_serial(&self) -> Result<std::os::unix::net::UnixStream>
    pub fn wait(self) -> Result<std::process::ExitStatus>
    pub fn kill(self) -> Result<()>
}
```

`run_tests` and `run_interactive` become thin wrappers over `spawn_vm` +
`VmHandle::connect_serial`. This is the seam that Phase B will use without
touching the xtask internals.

## Phase B Expectation (not designed here)

**Primary path: `KrunRuntime` via libkrun FFI.**

libkrun is a C library (Red Hat) that embeds a KVM-based microVM in-process — no QEMU
subprocess, no separate binary, ~125ms boot. It uses KVM on Linux and
Hypervisor.framework on macOS. `smolvm` is a thin CLI wrapper around libkrun; the
existing `macbox/src/krun/` path is already Phase 1 of this (shells out to smolvm,
with Phase 2 planned as direct FFI).

The Phase B adapter is therefore `KrunRuntime` in
`crates/minibox/src/adapters/krun.rs` (or promoted to a `krunbox` crate), implementing
`ContainerRuntime`. `MINIBOX_ADAPTER=krun` selects it on **both Linux and macOS** —
single adapter, two platforms, same domain port.

`MINIBOX_ADAPTER=qemu` remains a supported fallback for hosts without KVM/HVF or
where libkrun is not installed. `VmRunner::spawn_vm` backs the QEMU path.

Adapter selection precedence (Phase B):
1. `MINIBOX_ADAPTER=krun` — libkrun FFI (preferred, cross-platform)
2. `MINIBOX_ADAPTER=qemu` — QEMU subprocess fallback
3. `MINIBOX_ADAPTER=smolvm` — macOS only, shells out to smolvm CLI (existing)

## Files Touched (Phase A)

- `crates/xtask/src/vm_run.rs` — add `HostPlatform`, update free functions
- `crates/xtask/src/vm_image.rs` — replace `ALPINE_ARCH` constant, thread platform through
- `crates/xtask/src/main.rs` — detect platform once, pass to vm_run/vm_image callsites

## Files Touched (Phase C, follow-on)

- `crates/xtask/src/vm_run.rs` — refactor free functions into `VmRunner` + `VmHandle`
- `crates/xtask/src/main.rs` — update callsites to use `VmRunner`

## Testing

Phase A:
- Unit test `HostPlatform::detect()` with mocked `OS`/`ARCH` values
- `cargo xtask test-vm` on a Linux x86_64 CI runner (the acceptance test)

Phase C:
- `VmRunner::spawn_vm` + `VmHandle` unit tests using a mock QEMU binary
  (or gated behind `#[cfg(feature = "vm-tests")]`)
