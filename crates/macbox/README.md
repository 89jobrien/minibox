# macbox

macOS daemon implementation. Supports multiple adapter suites selected via `MINIBOX_ADAPTER`.

## Adapters

### `smolvm` (default)

Lightweight Linux VMs with subsecond boot. Selected automatically when the `smolvm` binary
is present on PATH. Falls back to `krun` when absent.

### `krun`

Uses libkrun to run containers in lightweight micro-VMs (HVF on macOS, KVM on Linux).
All four adapter ports (runtime, registry, filesystem, limiter) are wired and pass 31
conformance tests. Acts as the automatic fallback when smolvm is unavailable.

- `KrunRegistry`, `KrunRuntime`, `KrunFilesystem`, `KrunLimiter`

### `colima`

Uses Colima (limactl + nerdctl) for container management. All operations shell out to
CLIs running inside the Colima VM. Exec and logs are limited via Lima's SSH tunnel.

- `ColimaRegistry` — image pulling via nerdctl
- `ColimaRuntime` — container lifecycle via nerdctl
- `ColimaFilesystem` — overlay mount management
- `ColimaLimiter` — resource limit enforcement

Requires Colima to be running (`colima start`).

### `vz` (feature-gated, blocked)

Uses macOS Virtualization.framework to boot a lightweight Alpine Linux VM and forward
container operations to an in-VM miniboxd agent over vsock. Currently blocked by
`VZErrorInternal(code=1)` on macOS 26 ARM64 ([GH #61](https://github.com/89jobrien/minibox/issues/61)).

Requires `--features vz` and a VM image at `~/.minibox/vm/` (`cargo xtask build-vm-image`).

## Setup

```bash
# smolvm (default — no extra setup if smolvm binary is on PATH)
./target/release/miniboxd

# krun (explicit)
MINIBOX_ADAPTER=krun ./target/release/miniboxd

# Colima
colima start
MINIBOX_ADAPTER=colima ./target/release/miniboxd

# VZ (requires --features vz, currently blocked by Apple bug)
cargo xtask build-vm-image
cargo build --release --features vz
MINIBOX_ADAPTER=vz ./target/release/miniboxd
```
