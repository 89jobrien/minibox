# smolbox

`smolbox` is a compatibility facade for minibox's lightweight VM adapter suites. It preserves
stable import paths while the implementations remain owned by their platform crates:

- `smolbox::smolvm` re-exports smolvm types from `minibox::adapters`;
- `smolbox::krun` re-exports krun types from `macbox::krun`; and
- `smolbox::preflight` detects the external `smolvm` binary.

The crate is internal to the workspace (`publish = false`) and contains no daemon composition or
protocol implementation of its own.

## Re-exported API

The smolvm facade exports `SmolVmRegistry`, `SmolVmRuntime`, `SmolVmFilesystem`,
`SmolVmLimiter`, and `SmolVmExecutor`.

The krun facade exports `KrunRegistry`, `KrunRuntime`, `KrunFilesystem`, `KrunLimiter`, and
`SmolvmProcess`, both at the module root and through compatibility submodules.

```rust
use smolbox::krun::KrunRuntime;
use smolbox::preflight::{check_smolvm, smolvm_available};
use smolbox::smolvm::SmolVmRuntime;

let _smolvm_runtime = SmolVmRuntime::new();
let _krun_runtime = KrunRuntime::new();
let status = check_smolvm();
assert_eq!(status.found, smolvm_available());
```

## Preflight API

| API                      | Purpose                                                  |
| ------------------------ | -------------------------------------------------------- |
| `check_smolvm()`         | Resolve `smolvm` on `PATH` and query its version         |
| `smolvm_available()`     | Fast boolean `PATH` probe without a version subprocess   |
| `parse_version_output()` | Normalize supported `smolvm --version` formats           |
| `SmolvmStatus`           | Report `found`, parsed version, and resolved binary path |

A missing binary is normal and is returned as `SmolvmStatus { found: false, .. }`, not an error.
`miniboxd` performs its own probe when selecting the automatic default.

## Constraints and implementation status

- The facade does not install smolvm or libkrun.
- The smolvm adapter requires the external `smolvm` executable at runtime.
- The krun process path also delegates through the smolvm command wrapper.
- VM-backed suites currently lack Linux-native exec, bridge networking, pause/resume, and several
  image-management capabilities.
- Changing a re-export can break downstream imports even when the implementation type is unchanged.

## Development and testing

Tests cover preflight parsing/properties and verify that facade paths re-export the intended types.

```bash
cargo check -p smolbox
cargo clippy -p smolbox --all-targets -- -D warnings
cargo nextest run -p smolbox
cargo xtask test krun-conformance
```
