# smolbox

Compatibility import facade for VM adapter suites on macOS and Linux.

## Adapters

### smolvm

Re-exports the smolvm implementation owned by `minibox::adapters`.

### krun

Re-exports the krun implementation owned by `macbox::krun`.

## Preflight

The `preflight` module detects whether `smolvm` is installed and checks
its version, enabling the adapter registry in `miniboxd` to select the
correct suite at startup.

## Usage

smolbox is consumed by `miniboxd` and may be used by integrations that need
stable VM adapter import paths.

```toml
[dependencies]
smolbox = { path = "../smolbox" }
```

```rust
use smolbox::smolvm::SmolVmRuntime;
use smolbox::krun::KrunRuntime;
use smolbox::preflight::smolvm_available;
```
