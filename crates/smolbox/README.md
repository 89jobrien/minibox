# smolbox

Lightweight VM adapter suites for minibox on macOS and Linux.

## Adapters

### smolvm

Delegates container operations to the `smolvm` CLI (`smolvm machine run`).
Provides subsecond Linux VM boot on macOS via Apple Hypervisor.framework.
This is the default adapter when the `smolvm` binary is on PATH.

### krun

Uses libkrun to run containers inside micro-VMs (HVF on macOS, KVM on
Linux). Acts as the automatic fallback when smolvm is unavailable.
Implements all four domain ports: runtime, registry, filesystem, limiter.

## Preflight

The `preflight` module detects whether `smolvm` is installed and checks
its version, enabling the adapter registry in `miniboxd` to select the
correct suite at startup.

## Usage

smolbox is consumed by `macbox` and `miniboxd` — it is not used directly.

```toml
[dependencies]
smolbox = { path = "../smolbox" }
```

```rust
use smolbox::smolvm::SmolVmRuntime;
use smolbox::krun::KrunRuntime;
use smolbox::preflight::smolvm_available;
```
