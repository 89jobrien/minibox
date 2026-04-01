# macbox

macOS daemon implementation. Supports two adapter suites selected via `MINIBOX_ADAPTER`.

## Adapters

### `colima` (default)

Uses Colima (limactl + nerdctl) for container management. All operations shell out to CLIs running inside the Colima VM.

- `ColimaRegistry` — Image pulling via nerdctl
- `ColimaRuntime` — Container creation/lifecycle via nerdctl
- `ColimaFilesystem` — Overlay mount management
- `ColimaLimiter` — Resource limit enforcement (memory, CPU)

Requires Colima to be running (`colima start`).

### `vz` (feature-gated)

Uses macOS Virtualization.framework to boot a lightweight Alpine Linux VM and forward container operations to an in-VM miniboxd agent over vsock. Requires building with `--features vz` and a VM image at `~/.mbx/vm/` (build with `cargo xtask build-vm-image`).

- `VzRegistry`, `VzRuntime`, `VzFilesystem`, `VzLimiter` — trait impls forwarding via `VzProxy`
- `VzProxy` — JSON-over-vsock request/response to in-VM agent
- `VzVm` — VM boot/shutdown via objc2-virtualization bindings

## Setup

```bash
# Colima
colima start
MINIBOX_ADAPTER=colima ./target/release/miniboxd

# VZ (requires --features vz)
cargo xtask build-vm-image
cargo build --release --features vz
MINIBOX_ADAPTER=vz ./target/release/miniboxd
```
