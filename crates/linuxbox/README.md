# minibox

Re-export facade crate for the minibox container runtime.

Re-exports the public surface of `linuxbox` and `minibox-core` under the `minibox` crate name
so that consumers (integration tests, `daemonbox`, `miniboxd`) can depend on a single,
stable import path that survives internal renames.

## What lives here

| Upstream crate   | What is re-exported                                             |
| ---------------- | --------------------------------------------------------------- |
| `linuxbox`       | All public items (`pub use linuxbox::*`)                        |
| `minibox-core`   | Domain traits and protocol types (via `linuxbox` re-exports)    |

Direct code additions to this crate are intentionally kept to zero — the crate is a thin
shim only.

## Why this exists

The Linux container primitives live in `linuxbox` so that `miniboxd` can conditionally
compile them only on Linux while the `minibox` name remains stable for consumers. This
indirection also makes future renames transparent to downstream crates.

## Usage

```toml
[dependencies]
minibox = { path = "../minibox" }
```

```rust
use linuxbox::domain::ContainerRuntime;
use linuxbox::protocol::DaemonRequest;
```
