# minibox

Runtime application layer for the Minibox container runtime.

Contains Linux namespace setup, cgroups v2, overlay filesystem, cross-platform
daemon handler/server/state, and platform adapters. Also
re-exports `minibox-core` domain traits and protocol types.

## Key modules

| Module       | Purpose                                                      |
| ------------ | ------------------------------------------------------------ |
| `container/` | Linux namespace setup, cgroups v2, overlay FS, process spawn |
| `adapters/`  | Concrete adapter implementations of domain traits            |
| `daemon/`    | Handler, state machine, Unix socket server                   |

## Usage

```toml
[dependencies]
minibox = { path = "../minibox" }
```

```rust
use minibox::domain::ContainerRuntime;
use minibox::protocol::DaemonRequest;
```
